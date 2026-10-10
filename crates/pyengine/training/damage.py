"""Approximate damage-calc features in torch (TIPS: "the network shouldn't have to learn the damage formula").

Everything is vectorized over the batch so it runs on the training device. It is an estimate: the standard Gen 9
formula with STAB (including Tera), type effectiveness, expected ability/item immunities, weather, burn, screens,
stat stages, accuracy, multi-hit and a few variable-power moves. Opponent stats are estimated from base stats
(encoder.estimated_stats); the opponent's unrevealed moves are approximated by 80-power STAB attacks.

Outputs:

* ``action``: [B, 2, 40, N_ACTION_FEATURES] for every move code of both own active slots: expected damage dealt to
  the foes and to the own side (fraction of max HP, capped at the target's current HP), KO chances, the main
  target's effectiveness and whether the user moves first.
* ``token``: [B, 12, N_TOKEN_FEATURES] for every Pokemon: expected damage and KO chance from each opposing active,
  and whether it outspeeds each opposing active.
"""
from __future__ import annotations

import numpy as np
import torch
from torch import nn

from .dex import TARGETS, Dex
from .encoder import (FF, I_ABILITY, I_ITEM, I_MOVE0, I_SPECIES, I_TERA, I_TYPE1, N_ACTION_FEATURES,  # noqa: F401
                      N_TOKEN_FEATURES, PF, STAT_SCALE)

ROLL = 0.925  # mean damage roll
SPREAD = 0.75
OWN_TOKENS = (0, 1)
FOE_TOKENS = (6, 7)


def _hit_table() -> np.ndarray:
    """HIT[target class, actor slot, target code, token]: share of the move's damage that lands on each token."""
    table = np.zeros((len(TARGETS), 2, 5, 12), dtype=np.float32)
    chosen = {0: 1, 1: 0, 3: 6, 4: 7}  # target code -> token (code 0 = own slot b, 1 = own slot a)
    for c, name in enumerate(TARGETS):
        for s in (0, 1):
            ally = 1 - s
            if name in ("normal", "any", "adjacentFoe", "adjacentAlly", "adjacentAllyOrSelf"):
                for code, token in chosen.items():
                    table[c, s, code, token] = 1.0
            elif name == "allAdjacentFoes":
                table[c, s, 2, 6] = table[c, s, 2, 7] = SPREAD
            elif name == "allAdjacent":
                table[c, s, 2, 6] = table[c, s, 2, 7] = table[c, s, 2, ally] = SPREAD
            elif name == "randomNormal":
                table[c, s, 2, 6] = table[c, s, 2, 7] = 0.5
    return table


def stage(boost: torch.Tensor) -> torch.Tensor:
    return (2 + boost.clamp(min=0)) / (2 + (-boost).clamp(min=0))


class DamageFeatures(nn.Module):
    def __init__(self, dex: Dex):
        super().__init__()
        f = lambda a, dtype=torch.float32: torch.as_tensor(np.asarray(a), dtype=dtype)
        self.register_buffer("move_type", f(dex.move_type, torch.int64), persistent=False)
        self.register_buffer("move_category", f(dex.move_category, torch.int64), persistent=False)
        self.register_buffer("move_power", f(dex.move_power), persistent=False)
        self.register_buffer("move_accuracy", f(dex.move_accuracy), persistent=False)
        self.register_buffer("move_priority", f(dex.move_priority), persistent=False)
        self.register_buffer("move_target", f(dex.move_target, torch.int64), persistent=False)
        self.register_buffer("move_hits", f(dex.move_hits), persistent=False)
        self.register_buffer("move_special", f(dex.move_special_power, torch.int64), persistent=False)
        self.register_buffer("species_weight", f(dex.species_weight), persistent=False)
        self.register_buffer("typechart", f(dex.typechart), persistent=False)
        self.register_buffer("hit", f(_hit_table()), persistent=False)
        self.scarf = dex.item_index.get("choicescarf", -1)
        self.band = dex.item_index.get("choiceband", -1)
        self.specs = dex.item_index.get("choicespecs", -1)
        self.life_orb = dex.item_index.get("lifeorb", -1)
        self.huge_power = [dex.ability_index.get(k, -1) for k in ("hugepower", "purepower")]
        self.n_types = dex.n_types

    # ---- per-token attributes ------------------------------------------------------------------------------
    def tokens(self, ids: torch.Tensor, floats: torch.Tensor, field: torch.Tensor) -> dict:
        def col(name):
            return floats[..., PF[name]]
        stats = {s: col("stat_" + s) * STAT_SCALE for s in ("atk", "def", "spa", "spd", "spe")}
        terastallized = col("terastallized")
        tera = ids[..., I_TERA]
        base_types = ids[..., I_TYPE1:I_TYPE1 + 2]
        # Defensive types: the Tera type alone once terastallized (Stellar keeps the original types).
        tera_def = torch.stack((tera, torch.zeros_like(tera)), -1)
        stellar = tera == 19
        use_tera = (terastallized > 0.5) & (tera > 0) & ~stellar
        def_types = torch.where(use_tera.unsqueeze(-1), tera_def, base_types)
        ability, item = ids[..., I_ABILITY], ids[..., I_ITEM]
        atk = stats["atk"] * stage(col("boost_atk") * 6)
        huge = torch.zeros_like(atk, dtype=torch.bool)
        for a in self.huge_power:
            huge |= ability == a
        atk = torch.where(huge, atk * 2, atk)
        spe = stats["spe"] * stage(col("boost_spe") * 6) * torch.where(col("par") > 0, 0.5, 1.0)
        spe = torch.where(item == self.scarf, spe * 1.5, spe)
        own = col("own")
        own_tailwind = field[..., FF["own_tailwind"]].unsqueeze(-1)
        foe_tailwind = field[..., FF["foe_tailwind"]].unsqueeze(-1)
        spe = spe * (1 + torch.where(own > 0.5, own_tailwind, foe_tailwind))
        max_hp = col("stat_hp") * STAT_SCALE
        alive = col("present") * (1 - col("fainted"))
        reflect = torch.where(own > 0.5, field[..., FF["own_reflect"]].unsqueeze(-1), field[..., FF["foe_reflect"]].unsqueeze(-1))
        light = torch.where(own > 0.5, field[..., FF["own_lightscreen"]].unsqueeze(-1), field[..., FF["foe_lightscreen"]].unsqueeze(-1))
        veil = torch.where(own > 0.5, field[..., FF["own_auroraveil"]].unsqueeze(-1), field[..., FF["foe_auroraveil"]].unsqueeze(-1))
        orb = torch.where(item == self.life_orb, 1.3, 1.0)
        item_phys = torch.where(item == self.band, 1.5, 1.0) * orb
        item_spec = torch.where(item == self.specs, 1.5, 1.0) * orb
        return dict(
            item_phys=item_phys, item_spec=item_spec, level=col("level") * 100, atk=atk, spa=stats["spa"] * stage(col("boost_spa") * 6),
            dfn=stats["def"] * stage(col("boost_def") * 6), spd=stats["spd"] * stage(col("boost_spd") * 6),
            spe=spe, hp=col("hp") * max_hp, hp_frac=col("hp"), max_hp=max_hp.clamp(min=1), alive=alive,
            burned=col("brn"), base_types=base_types, def_types=def_types, tera=tera,
            terastallized=terastallized, type_mult=floats[..., PF["type_mult0"]:PF["type_mult0"] + 20],
            weight=self.species_weight[ids[..., I_SPECIES]], item=item,
            screen_phys=torch.maximum(reflect, veil), screen_spec=torch.maximum(light, veil),
        )

    # ---- core formula --------------------------------------------------------------------------------------
    def damage(self, att: dict, dfn: dict, mtype, category, power, accuracy, hits, special, field):
        """att tensors are [B, A, 1, 1], move tensors [B, A, M, 1], dfn tensors [B, 1, 1, D]; returns
        expected dealt fraction of the defender's max HP (capped at its current HP), KO chance and effectiveness,
        each [B, A, M, D]."""
        physical = category == 1
        attack = torch.where(physical, att["atk"] * att["item_phys"], att["spa"] * att["item_spec"])
        defense = torch.where(physical, dfn["dfn"], dfn["spd"]).clamp(min=1)
        # Variable power: weight-based (1, 2) and HP-based (3).
        w_def, w_att = dfn["weight"], att["weight"]
        weight_power = torch.where(w_def < 10, 20.0, torch.where(w_def < 25, 40.0, torch.where(w_def < 50, 60.0,
                       torch.where(w_def < 100, 80.0, torch.where(w_def < 200, 100.0, 120.0)))))
        ratio = w_att / w_def.clamp(min=0.1)
        ratio_power = torch.where(ratio >= 5, 120.0, torch.where(ratio >= 4, 100.0, torch.where(ratio >= 3, 80.0,
                      torch.where(ratio >= 2, 60.0, 40.0))))
        power = torch.where(special == 1, weight_power, power)
        power = torch.where(special == 2, ratio_power, power)
        power = torch.where(special == 3, 150 * att["hp_frac"], power)
        base = (2 * att["level"] / 5 + 2) * power * attack / defense / 50 + 2
        tera_match = (att["terastallized"] > 0.5) & (att["tera"] == mtype)
        orig_match = (att["base_types"][..., 0] == mtype) | (att["base_types"][..., 1] == mtype)
        stab = 1 + 0.5 * orig_match.float() + 0.5 * tera_match.float()
        n = self.n_types
        flat = self.typechart.reshape(-1)
        eff = flat[mtype * n + dfn["def_types"][..., 0]] * flat[mtype * n + dfn["def_types"][..., 1]]
        type_onehot = (mtype.unsqueeze(-1) == torch.arange(20, device=mtype.device)).float()  # [B, A, M, 1, 20]
        immune_mult = (type_onehot * dfn["type_mult"]).sum(-1)
        sun, rain = field[:, FF["sun"]].view(-1, 1, 1, 1), field[:, FF["rain"]].view(-1, 1, 1, 1)
        fire, water = (mtype == 10).float(), (mtype == 11).float()
        weather = 1 + sun * (0.5 * fire - 0.5 * water) + rain * (0.5 * water - 0.5 * fire)
        burn = torch.where(physical, 1 - 0.5 * att["burned"], torch.ones_like(att["burned"]))
        screen = 1 - torch.where(physical, dfn["screen_phys"], dfn["screen_spec"]) / 3
        damage = base * stab * eff * immune_mult * weather * burn * screen * ROLL * hits
        damage = torch.where(special == 4, att["level"] * (eff > 0).float(), damage)  # Night Shade
        damage = torch.where(special == 5, dfn["hp"] / 2 * (eff > 0).float(), damage)  # Super Fang, Ruination
        damage = torch.where((special == 6) | (category == 0), torch.zeros_like(damage), damage)
        top = damage / ROLL
        ko = ((top - dfn["hp"]) / (0.15 * top).clamp(min=1e-3)).clamp(0, 1) * accuracy
        dealt = torch.minimum(damage, dfn["hp"]) / dfn["max_hp"] * accuracy
        alive = dfn["alive"]
        return dealt * alive, ko * alive, eff * immune_mult

    def forward(self, ids: torch.Tensor, floats: torch.Tensor, field: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        B = ids.shape[0]
        t = self.tokens(ids, floats, field)
        trick_room = field[:, FF["trickroom"]].view(B, 1, 1)

        defenders = {k: v.reshape(B, 1, 1, 12, *v.shape[2:]) for k, v in t.items()}

        # Own actives with their real moves against every token.
        own = {k: v[:, list(OWN_TOKENS)].reshape(B, 2, 1, 1, *v.shape[2:]) for k, v in t.items()}
        moves = ids[:, list(OWN_TOKENS), I_MOVE0:I_MOVE0 + 4]  # [B, 2, 4]
        mv = lambda table: table[moves].unsqueeze(-1)  # [B, 2, 4, 1]
        own_dealt, own_ko, own_eff = self.damage(own, defenders, mv(self.move_type), mv(self.move_category),
                                                 mv(self.move_power), mv(self.move_accuracy), mv(self.move_hits),
                                                 mv(self.move_special), field)  # [B, 2, 4, 12]
        # Foe actives: revealed moves plus 80-power STAB attacks of each type with their better attacking stat.
        foe = {k: v[:, list(FOE_TOKENS)].reshape(B, 2, 1, 1, *v.shape[2:]) for k, v in t.items()}
        foe_moves = ids[:, list(FOE_TOKENS), I_MOVE0:I_MOVE0 + 4]
        stab_types = torch.cat((t["base_types"][:, list(FOE_TOKENS)], t["tera"][:, list(FOE_TOKENS)].unsqueeze(-1)
                                * (t["terastallized"][:, list(FOE_TOKENS)] > 0.5).long().unsqueeze(-1)), -1)  # [B,2,3]
        better = torch.where(t["atk"][:, list(FOE_TOKENS)] >= t["spa"][:, list(FOE_TOKENS)], 1, 2).unsqueeze(-1)
        generic_cat = better.expand(-1, -1, 3) * (stab_types > 0).long()
        f_type = torch.cat((self.move_type[foe_moves], stab_types), -1).unsqueeze(-1)
        f_cat = torch.cat((self.move_category[foe_moves], generic_cat), -1).unsqueeze(-1)
        f_pow = torch.cat((self.move_power[foe_moves], torch.full_like(stab_types, 80, dtype=torch.float32)), -1).unsqueeze(-1)
        f_acc = torch.cat((self.move_accuracy[foe_moves], torch.ones_like(stab_types, dtype=torch.float32)), -1).unsqueeze(-1)
        f_hits = torch.cat((self.move_hits[foe_moves], torch.ones_like(stab_types, dtype=torch.float32)), -1).unsqueeze(-1)
        f_spec = torch.cat((self.move_special[foe_moves], torch.zeros_like(stab_types)), -1).unsqueeze(-1)
        foe_dealt, foe_ko, _ = self.damage(foe, defenders, f_type, f_cat, f_pow, f_acc, f_hits, f_spec, field)  # [B,2,7,12]

        # Speed: does each token outspeed each opposing active (Trick Room flips).
        spe = t["spe"]
        own_spe, foe_spe = spe[:, list(OWN_TOKENS)], spe[:, list(FOE_TOKENS)]
        faster_foe = (spe.unsqueeze(-1) > foe_spe.unsqueeze(1)).float() + 0.5 * (spe.unsqueeze(-1) == foe_spe.unsqueeze(1)).float()
        faster_own = (spe.unsqueeze(-1) > own_spe.unsqueeze(1)).float() + 0.5 * (spe.unsqueeze(-1) == own_spe.unsqueeze(1)).float()
        faster_foe = torch.where(trick_room > 0.5, 1 - faster_foe, faster_foe)  # [B, 12, 2]
        faster_own = torch.where(trick_room > 0.5, 1 - faster_own, faster_own)
        is_own = (floats[..., PF["own"]] > 0.5).unsqueeze(-1)  # [B, 12, 1]
        threat_from_foe = foe_dealt.amax(2).transpose(1, 2)  # [B, 12, 2]
        ko_from_foe = foe_ko.amax(2).transpose(1, 2)
        threat_from_own = own_dealt.amax(2).transpose(1, 2)
        ko_from_own = own_ko.amax(2).transpose(1, 2)
        token = torch.cat((
            torch.where(is_own, threat_from_foe, threat_from_own),
            torch.where(is_own, ko_from_foe, ko_from_own),
            torch.where(is_own, faster_foe, faster_own),
        ), -1) * floats[..., PF["present"]].unsqueeze(-1)

        # Per move code: weight damage by the tokens the code hits.
        hit = self.hit[self.move_target[moves]]  # [B, 2, 4, 2(actor), 5, 12]
        hit = torch.stack((hit[:, 0, :, 0], hit[:, 1, :, 1]), 1)  # [B, 2, 4, 5, 12]
        dealt = hit * own_dealt.unsqueeze(3)
        kos = hit * own_ko.unsqueeze(3)
        foe_mask = torch.zeros(12, device=ids.device)
        foe_mask[6:12] = 1
        own_mask = 1 - foe_mask
        main = (hit.argmax(-1, keepdim=True) == torch.arange(12, device=ids.device)).float()  # [B, 2, 4, 5, 12]
        eff_main = (main * own_eff.unsqueeze(3)).sum(-1)
        any_hit = (hit.sum(-1) > 0).float()
        priority = self.move_priority[moves].unsqueeze(-1)  # [B, 2, 4, 1]
        first = torch.stack((faster_foe[:, 0], faster_foe[:, 1]), 1).unsqueeze(2)  # [B, 2, 1, 2]
        first = torch.where(priority > 0, torch.ones_like(first), torch.where(priority < 0, torch.zeros_like(first), first))
        action = torch.stack((
            (dealt * foe_mask).sum(-1), (dealt * own_mask).sum(-1),
            (kos * foe_mask).sum(-1), (kos * own_mask).sum(-1),
            torch.log2(eff_main.clamp(min=0.125)) / 2 * any_hit, (eff_main == 0).float() * any_hit,
            first[..., 0:1].expand(-1, -1, -1, 5), first[..., 1:2].expand(-1, -1, -1, 5),
            (priority / 5).expand(-1, -1, -1, 5),
        ), -1)  # [B, 2, 4, 5, F]
        action = action.unsqueeze(4).expand(-1, -1, -1, -1, 2, -1).reshape(B, 2, 40, N_ACTION_FEATURES)
        return action, token
