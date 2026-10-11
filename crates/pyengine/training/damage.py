"""Approximate damage-calc features in torch (TIPS: "the network shouldn't have to learn the damage formula").

Everything is vectorized over the batch so it runs on the training device. It is an estimate: the standard Gen 9
formula with STAB (including Tera), type effectiveness, expected ability/item immunities, weather, burn, screens,
stat stages, accuracy, multi-hit, spread reduction and a few variable-power moves. Opponent stats are estimated from
base stats (encoder.estimated_stats). An opponent's possible moves are its revealed moves (probability 1) plus the
belief candidates of training.dex.Dex.belief with their inclusion probabilities; only when both are empty do
80-power STAB attacks of its types stand in.

Outputs:

* ``action``: [B, 2, 40, N_ACTION_FEATURES] for every move code of both own active slots: expected damage dealt to
  the foes and to the own side (fraction of max HP, capped at the target's current HP), KO chances, the main
  target's effectiveness and whether the user moves first. These nine columns are the same for a code with and
  without Tera. Then what the action does beyond its damage (``ACTION_COLUMNS``):

  - ``fail``: the move fails outright: Fake Out and First Impression after the user's first move action since it
    switched in, or a priority move at a foe that Psychic Terrain (grounded foes) or Armor Tail, Dazzling or Queenly
    Majesty (on the foe side) protects.
  - ``flinch_a``/``flinch_b``: chance that the move flinches foe a/b before it acts (flinch chance, accuracy, moving
    first, and the foe can flinch: no Inner Focus, Shield Dust, Covert Cloak or Substitute, not immune).
  - ``stop_damage``/``stop_ko``: the damage and KO chance those flinches prevent this turn (each foe's best threat
    against our actives).
  - ``protect``: success chance of a Protect-like move (1/3 right after a successful one), ``protect_damage``/
    ``protect_ko`` that times the damage and KO chance the user faces this turn.
  - ``foe_protect_a``/``foe_protect_b``: chance that foe a/b can Protect against the move (it has a Protect move,
    revealed or by the set belief, at full or 1/3 success; moves without the ``protect`` flag and Unseen Fist
    contact moves get 0). This is "can", not "will".
  - Tera codes only: ``tera_damage``/``tera_ko``, the change of damage and KO chance on the foes from terastallizing
    (STAB), and ``tera_threat``/``tera_threat_ko``, the change of the damage and KO chance the user faces this turn
    (its defensive type). Zero without Tera.
  - Rust-only: ``protect_chance_a``/``protect_chance_b`` estimate whether each targeted foe protects this turn
    from its ability to Protect and the damage/KO threat from our actives. ``damage_after_protect``/
    ``ko_after_protect`` discount the legacy damage/KO shares by those chances. These four columns are zero
    in this frozen Python reference, and identical with and without Tera in Rust.

* ``token``: [B, 12, N_TOKEN_FEATURES] for every Pokemon: expected damage and KO chance from each opposing active,
  and whether it outspeeds each opposing active. For damage from an opponent, each possible move is assumed present
  independently with its probability and the opponent to use its best present move: the expected damage is
  E[max] = sum_i d_i p_i prod_{j<i} (1 - p_j) over moves sorted by damage, and the KO chance 1 - prod (1 - p_m KO_m).
  Then (``TOKEN_COLUMNS``): ``fake_out_ready`` (active, has not used a move action since switching in, and has Fake
  Out: known, or by the set belief), ``flinchable`` (a Fake Out on it would flinch it), ``fake_out_threat`` (an
  opposing active can Fake Out it this turn) and ``protect_ready`` (active and can Protect successfully).
  Rust-only: ``offense_a``/``offense_b`` and ``offense_ko_a``/``offense_ko_b`` give each own Pokemon's best
  damage/KO chance against the foe actives, including bench Pokemon as if on the field. ``entry_damage`` gives
  own-side hazard damage on entry (Boots/Magic Guard, grounding and current HP included). ``protect_now`` gives
  the foe actives' Protect prior before per-move blocking. These six columns are zero in the Python reference.

The first LEGACY_ACTION_FEATURES and LEGACY_TOKEN_FEATURES columns are exactly the first run's features, so older
models still run (training.model reads only the columns a model was built with).
"""
from __future__ import annotations

import numpy as np
import torch
from torch import nn

from .dex import TARGETS, Dex
from .encoder import (FF, I_ABILITY, I_CAND_ABILITIES, I_CAND_ITEMS, I_CAND_MOVES, I_ITEM,  # noqa: F401
                      I_MOVE0, I_SPECIES, I_TERA, I_TYPE1, K_ABILITIES, K_ITEMS, K_MOVES, LEGACY_ACTION_FEATURES,
                      LEGACY_TOKEN_FEATURES, N_ACTION_FEATURES, N_TOKEN_FEATURES, PF, REFERENCE_ACTION_FEATURES,
                      REFERENCE_TOKEN_FEATURES, STAT_SCALE)
from .guards import FIRST_TURN_ONLY

ROLL = 0.925  # mean damage roll
SPREAD = 0.75
OWN_TOKENS = (0, 1)
FOE_TOKENS = (6, 7)
FLINCH_BLOCK = ("innerfocus", "shielddust")
PRIORITY_BLOCK = ("armortail", "dazzling", "queenlymajesty")
REPEAT_PROTECT = 1 / 3  # success chance of a Protect-like move right after a successful one (1/9 after two: not tracked)
FLINCH_CHANCE = {"triplearrows": 0.3}  # dex.json's secondaryChance is the largest of a move's secondaries
MOLD_BREAKER = ("moldbreaker", "teravolt", "turboblaze")
STELLAR = 19
ACTION_COLUMNS = ("fail", "flinch_a", "flinch_b", "stop_damage", "stop_ko", "protect", "protect_damage", "protect_ko",
                  "foe_protect_a", "foe_protect_b", "tera_damage", "tera_ko", "tera_threat", "tera_threat_ko",
                  "protect_chance_a", "protect_chance_b", "damage_after_protect", "ko_after_protect")
TOKEN_COLUMNS = ("fake_out_ready", "flinchable", "fake_out_threat", "protect_ready", "offense_a", "offense_b",
                 "offense_ko_a", "offense_ko_b", "entry_damage", "protect_now")
assert LEGACY_ACTION_FEATURES + len(ACTION_COLUMNS) == N_ACTION_FEATURES
assert LEGACY_TOKEN_FEATURES + len(TOKEN_COLUMNS) == N_TOKEN_FEATURES


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


def expected_max(dealt: torch.Tensor, probs: torch.Tensor) -> torch.Tensor:
    """E[max over present moves] of ``dealt`` [B, A, M, D] when move m is present independently with ``probs``
    [B, A, M]; absent moves deal nothing. Reduces the move axis."""
    p = probs.clamp(0, 1).unsqueeze(-1).expand_as(dealt)
    order = dealt.argsort(dim=2, descending=True)
    d, q = dealt.gather(2, order), p.gather(2, order)
    survive = torch.cumprod(1 - q, dim=2)
    before = torch.cat((torch.ones_like(survive[:, :, :1]), survive[:, :, :-1]), 2)
    return (d * q * before).sum(2)


def any_ko(ko: torch.Tensor, probs: torch.Tensor) -> torch.Tensor:
    """Chance that at least one present move KOs: 1 - prod_m (1 - p_m KO_m), reducing the move axis."""
    return 1 - torch.prod(1 - probs.clamp(0, 1).unsqueeze(-1) * ko, dim=2)


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
        # Effects beyond damage (the ACTION_COLUMNS and TOKEN_COLUMNS).
        n = dex.n_moves
        flinch, first_only, blockable, contact = (np.zeros(n, dtype=np.float32) for _ in range(4))
        for i, m in enumerate(dex.moves, start=1):
            flinch[i] = FLINCH_CHANCE.get(m["id"], m["secondaryChance"] if m["secondaryVolatile"] == "flinch" else 0.0)
            first_only[i] = m["id"] in FIRST_TURN_ONLY
            blockable[i] = "protect" in m["flags"]
            contact[i] = "contact" in m["flags"]
        self.register_buffer("move_flinch", f(flinch), persistent=False)
        self.register_buffer("move_first_only", f(first_only), persistent=False)
        self.register_buffer("move_blockable", f(blockable), persistent=False)
        self.register_buffer("move_contact", f(contact), persistent=False)
        self.register_buffer("move_protect", f(np.asarray(dex.move_stalling, dtype=np.float32)), persistent=False)
        fake_out = np.zeros(n, dtype=np.float32)
        if "fakeout" in dex.move_index:
            fake_out[dex.move_index["fakeout"]] = 1
        self.register_buffer("move_fake_out", f(fake_out), persistent=False)
        ability_table = lambda names: f([0.0] + [float(a["id"] in names) for a in dex.abilities])  # noqa: E731
        self.register_buffer("ability_flinch_block", ability_table(FLINCH_BLOCK), persistent=False)
        self.register_buffer("ability_priority_block", ability_table(PRIORITY_BLOCK), persistent=False)
        self.register_buffer("ability_levitate", ability_table(("levitate",)), persistent=False)
        self.register_buffer("ability_unseen_fist", ability_table(("unseenfist",)), persistent=False)
        self.register_buffer("ability_mold_breaker", ability_table(MOLD_BREAKER), persistent=False)
        self.ability = {name: dex.ability_index.get(name, -1) for name in
                        ("sheerforce", "serenegrace", "prankster", "galewings")}
        self.tera_blast = dex.move_index.get("terablast", -1)
        self.grassy_glide = dex.move_index.get("grassyglide", -1)
        self.covert_cloak = dex.item_index.get("covertcloak", -1)
        self.air_balloon = dex.item_index.get("airballoon", -1)
        self.flying, self.normal, self.dark = dex.type_idx("Flying"), dex.type_idx("Normal"), dex.type_idx("Dark")

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
            stellar=((terastallized > 0.5) & stellar).float(),  # Stellar STAB once Stellar-terastallized (damage())
            weight=self.species_weight[ids[..., I_SPECIES]], item=item,
            screen_phys=torch.maximum(reflect, veil), screen_spec=torch.maximum(light, veil),
        )

    # ---- core formula --------------------------------------------------------------------------------------
    def damage(self, att: dict, dfn: dict, mtype, category, power, accuracy, hits, special, field, modifier=None):
        """att tensors are [B, A, 1, 1], move tensors [B, A, M, 1], dfn tensors [B, 1, 1, D]; returns
        expected dealt fraction of the defender's max HP (capped at its current HP), KO chance and effectiveness,
        each [B, A, M, D]. ``modifier`` [B, A, M, 1] scales the damage (spread moves)."""
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
        # Stellar Tera: 2x on the original types' moves and 1.2x on the others (every use: random battles give
        # Stellar only to Terapagos, whose boost is not used up).
        stab = torch.where(att["stellar"] > 0.5, torch.where(orig_match, 2.0, 1.2), stab)
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
        if modifier is not None:
            damage = damage * modifier
        top = damage / ROLL
        ko = ((top - dfn["hp"]) / (0.15 * top).clamp(min=1e-3)).clamp(0, 1) * accuracy
        dealt = torch.minimum(damage, dfn["hp"]) / dfn["max_hp"] * accuracy
        alive = dfn["alive"]
        return dealt * alive, ko * alive, eff * immune_mult

    def foe_attacks(self, ids, floats, t, defenders, field):
        """Damage of every possible move of both foe actives against every token, with the move probabilities.

        Moves: the 4 revealed slots (probability 1 where revealed), the K_MOVES belief candidates (their inclusion
        probabilities), then three 80-power STAB attacks with the better attacking stat (probability 1 only when the
        revealed and belief moves are all empty, for example a species without random-battle data).
        """
        B = ids.shape[0]
        foe_tokens = list(FOE_TOKENS)
        foe = {k: v[:, foe_tokens].reshape(B, 2, 1, 1, *v.shape[2:]) for k, v in t.items()}
        moves = torch.cat((ids[:, foe_tokens, I_MOVE0:I_MOVE0 + 4], ids[:, foe_tokens, I_CAND_MOVES:I_CAND_MOVES + K_MOVES]), -1)
        probs = torch.cat((floats[:, foe_tokens, PF["move_known0"]:PF["move_known0"] + 4],
                           floats[:, foe_tokens, PF["cand_move_p0"]:PF["cand_move_p0"] + K_MOVES]), -1) * (moves > 0)
        on_field = t["alive"][:, foe_tokens].unsqueeze(-1)  # a fainted foe still in its slot attacks no one
        stab_types = torch.cat((t["base_types"][:, foe_tokens], t["tera"][:, foe_tokens].unsqueeze(-1)
                                * (t["terastallized"][:, foe_tokens] > 0.5).long().unsqueeze(-1)), -1)  # [B, 2, 3]
        better = torch.where(t["atk"][:, foe_tokens] >= t["spa"][:, foe_tokens], 1, 2).unsqueeze(-1)
        generic_cat = better.expand(-1, -1, 3) * (stab_types > 0).long()
        fallback = (probs.sum(-1, keepdim=True) == 0).float() * (stab_types > 0).float()
        target = self.move_target[moves]
        spread = (target == TARGETS.index("allAdjacentFoes")) | (target == TARGETS.index("allAdjacent"))
        ones = torch.ones_like(stab_types, dtype=torch.float32)
        cat = lambda a, b: torch.cat((a, b), -1).unsqueeze(-1)
        dealt, ko, _ = self.damage(
            foe, defenders, cat(self.move_type[moves], stab_types), cat(self.move_category[moves], generic_cat),
            cat(self.move_power[moves], 80 * ones), cat(self.move_accuracy[moves], ones),
            cat(self.move_hits[moves], ones), cat(self.move_special[moves], torch.zeros_like(stab_types)), field,
            modifier=cat(torch.where(spread, SPREAD, 1.0), ones))
        return dealt, ko, torch.cat((probs, fallback), -1) * on_field

    def forward(self, ids: torch.Tensor, floats: torch.Tensor, field: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        B = ids.shape[0]
        t = self.tokens(ids, floats, field)
        trick_room = field[:, FF["trickroom"]].view(B, 1, 1)

        defenders = {k: v.reshape(B, 1, 1, 12, *v.shape[2:]) for k, v in t.items()}
        # The own actives as if they terastallized now ride along as attackers 2-3 and defenders 12-13 (the damage
        # formula is elementwise, so the other entries are unchanged).
        tera_t = self._terastallized(t)

        # Own actives with their real moves against every token.
        own = {k: torch.cat((v[:, list(OWN_TOKENS)], tera_t[k]), 1).reshape(B, 4, 1, 1, *v.shape[2:])
               for k, v in t.items()}
        moves = ids[:, list(OWN_TOKENS), I_MOVE0:I_MOVE0 + 4]  # [B, 2, 4]
        mv = lambda table: table[torch.cat((moves, moves), 1)].unsqueeze(-1)  # [B, 4, 4, 1]
        # Tera Blast takes the Tera type, and is physical when Attack is higher.
        blast = moves == self.tera_blast
        typed = blast & ((tera_t["tera"] > 0) & (tera_t["tera"] != STELLAR)).unsqueeze(-1)
        move_type = self.move_type[moves]
        move_type = torch.cat((move_type, torch.where(typed, tera_t["tera"].unsqueeze(-1), move_type)), 1)
        category = self.move_category[moves]
        blast_category = torch.where(tera_t["atk"] > tera_t["spa"], 1, 2).unsqueeze(-1)
        category = torch.cat((category, torch.where(blast, blast_category, category)), 1)
        all_dealt, all_ko, all_eff = self.damage(own, defenders, move_type.unsqueeze(-1), category.unsqueeze(-1),
                                                 mv(self.move_power), mv(self.move_accuracy), mv(self.move_hits),
                                                 mv(self.move_special), field)  # [B, 4, 4, 12]
        own_dealt, own_ko, own_eff = all_dealt[:, :2], all_ko[:, :2], all_eff[:, :2]
        targets = {k: torch.cat((v, tera_t[k]), 1).reshape(B, 1, 1, 14, *v.shape[2:]) for k, v in t.items()}
        foe_dealt, foe_ko, foe_probs = self.foe_attacks(ids, floats, t, targets, field)  # [B, 2, M, 14], [B, 2, M]

        # Speed: does each token outspeed each opposing active (Trick Room flips).
        spe = t["spe"]
        own_spe, foe_spe = spe[:, list(OWN_TOKENS)], spe[:, list(FOE_TOKENS)]
        faster_foe = (spe.unsqueeze(-1) > foe_spe.unsqueeze(1)).float() + 0.5 * (spe.unsqueeze(-1) == foe_spe.unsqueeze(1)).float()
        faster_own = (spe.unsqueeze(-1) > own_spe.unsqueeze(1)).float() + 0.5 * (spe.unsqueeze(-1) == own_spe.unsqueeze(1)).float()
        faster_foe = torch.where(trick_room > 0.5, 1 - faster_foe, faster_foe)  # [B, 12, 2]
        faster_own = torch.where(trick_room > 0.5, 1 - faster_own, faster_own)
        is_own = (floats[..., PF["own"]] > 0.5).unsqueeze(-1)  # [B, 12, 1]
        threat_all = expected_max(foe_dealt, foe_probs).transpose(1, 2)  # [B, 14, 2]
        ko_all = any_ko(foe_ko, foe_probs).transpose(1, 2)
        threat_from_foe, ko_from_foe = threat_all[:, :12], ko_all[:, :12]  # [B, 12, 2]
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
        # Main target: the first token with the largest share, a foe on ties (Earthquake and Surf hit the ally too).
        tie_break = hit + 1e-3 * foe_mask * (hit > 0).float()
        main = (tie_break.argmax(-1, keepdim=True) == torch.arange(12, device=ids.device)).float()  # [B, 2, 4, 5, 12]
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
        ), -1)  # [B, 2, 4, 5, LEGACY_ACTION_FEATURES]
        p = self._per_token(ids, floats, field, t)
        # What still lands through Protect: moves without the protect flag, and Unseen Fist contact moves.
        foe_moves = torch.cat((ids[:, list(FOE_TOKENS), I_MOVE0:I_MOVE0 + 4],
                               ids[:, list(FOE_TOKENS), I_CAND_MOVES:I_CAND_MOVES + K_MOVES]), -1)  # [B, 2, M - 3]
        unseen = p["unseen_fist"][:, list(FOE_TOKENS)].unsqueeze(-1)
        bypass = 1 - self.move_blockable[foe_moves] * (1 - unseen * self.move_contact[foe_moves])
        bypass = torch.cat((bypass, torch.zeros_like(bypass[..., :3])), -1).unsqueeze(-1)  # generic attacks: blocked
        users = list(OWN_TOKENS)
        through = expected_max(foe_dealt[..., users] * bypass, foe_probs).transpose(1, 2)  # [B, 2 users, 2 foes]
        through_ko = any_ko(foe_ko[..., users] * bypass, foe_probs).transpose(1, 2)
        effects = self.effects(ids, field, t, p, moves, hit, faster_foe, own_eff, threat_from_foe, ko_from_foe,
                               through, through_ko)
        tera = self.tera_effects(floats, p, hit, foe_mask, own_dealt, own_ko, all_dealt[:, 2:], all_ko[:, 2:],
                                 threat_from_foe, ko_from_foe, threat_all[:, 12:], ko_all[:, 12:])
        both = torch.cat((action, effects), -1).unsqueeze(4).expand(-1, -1, -1, -1, 2, -1)  # same with and without Tera
        tera = torch.stack((torch.zeros_like(tera), tera), 4)  # Tera columns only on Tera codes
        action = torch.cat((both, tera), -1).reshape(B, 2, 40, REFERENCE_ACTION_FEATURES)
        token = torch.cat((token, self.token_effects(ids, floats, t, p)), -1)
        action = torch.cat((action, action.new_zeros(B, 2, 40, N_ACTION_FEATURES - REFERENCE_ACTION_FEATURES)), -1)
        token = torch.cat((token, token.new_zeros(B, 12, N_TOKEN_FEATURES - REFERENCE_TOKEN_FEATURES)), -1)
        return action, token

    # ---- effects beyond damage ---------------------------------------------------------------------------------
    @staticmethod
    def _terastallized(t: dict) -> dict:
        """The own actives' attributes ([B, 2, ...]) as ``tokens`` gives them once terastallized: Tera STAB, and the
        Tera type alone as the defensive type (Stellar keeps the original types)."""
        own = {k: v[:, list(OWN_TOKENS)] for k, v in t.items()}
        tera = own["tera"]
        use = ((tera > 0) & (tera != STELLAR)).unsqueeze(-1)
        own["def_types"] = torch.where(use, torch.stack((tera, torch.zeros_like(tera)), -1), own["base_types"])
        own["terastallized"] = torch.ones_like(own["terastallized"])
        own["stellar"] = (tera == STELLAR).float()
        return own

    def _has_move(self, ids, floats, table):
        """[B, 12]: chance that each Pokemon has a move of ``table`` (revealed or known: 1; else the set belief)."""
        known = (table[ids[..., I_MOVE0:I_MOVE0 + 4]] * floats[..., PF["move_known0"]:PF["move_known0"] + 4]).amax(-1)
        believed = (table[ids[..., I_CAND_MOVES:I_CAND_MOVES + K_MOVES]]
                    * floats[..., PF["cand_move_p0"]:PF["cand_move_p0"] + K_MOVES]).sum(-1).clamp(max=1)
        return known + (1 - known) * believed

    def _ability_p(self, ids, floats, table):
        """[B, 12]: chance that each Pokemon's ability is in ``table`` (known, else the belief over candidates)."""
        ability = ids[..., I_ABILITY]
        believed = (table[ids[..., I_CAND_ABILITIES:I_CAND_ABILITIES + K_ABILITIES]]
                    * floats[..., PF["cand_ability_p0"]:PF["cand_ability_p0"] + K_ABILITIES]).sum(-1).clamp(max=1)
        return torch.where(ability > 0, table[ability], believed)

    def _item_p(self, ids, floats, item):
        """[B, 12]: chance that each Pokemon holds ``item`` (known, else the belief over candidates)."""
        held = ids[..., I_ITEM]
        believed = ((ids[..., I_CAND_ITEMS:I_CAND_ITEMS + K_ITEMS] == item).float()
                    * floats[..., PF["cand_item_p0"]:PF["cand_item_p0"] + K_ITEMS]).sum(-1).clamp(max=1)
        return torch.where(held > 0, (held == item).float(), believed)

    def _per_token(self, ids, floats, field, t) -> dict:
        """Per-Pokemon state shared by the effect columns, all [B, 12]."""
        col = lambda name: floats[..., PF[name]]  # noqa: E731
        on = t["alive"] * ((col("active_a") + col("active_b")) > 0.5).float()
        own = col("own") > 0.5
        # A Fake Out on it would flinch it: ability (Mold Breaker ignores it), Covert Cloak, Substitute.
        flinch_ability_ok = 1 - self._ability_p(ids, floats, self.ability_flinch_block)
        flinch_other_ok = (1 - self._item_p(ids, floats, self.covert_cloak)) * (1 - col("substitute"))
        # Priority moves from the other side: Psychic Terrain (grounded) and the side's Armor Tail, Dazzling, Queenly
        # Majesty.
        flying = (t["def_types"] == self.flying).any(-1).float()
        airborne = 1 - (1 - flying) * (1 - self._ability_p(ids, floats, self.ability_levitate)) * (
            1 - self._item_p(ids, floats, self.air_balloon))
        grounded = torch.where(field[:, FF["gravity"]:FF["gravity"] + 1] > 0.5, torch.ones_like(airborne), 1 - airborne)
        blocker = self._ability_p(ids, floats, self.ability_priority_block) * on
        side_block = torch.stack((1 - torch.prod(1 - blocker[:, 0:2], -1), 1 - torch.prod(1 - blocker[:, 6:8], -1)), -1)
        side_block = torch.where(own, side_block[:, 0:1], side_block[:, 1:2])
        priority_ok = (1 - field[:, FF["psychicterrain"]:FF["psychicterrain"] + 1] * grounded) * (1 - side_block)
        repeat = 1 - (1 - REPEAT_PROTECT) * col("protect_last")
        fresh = col("fresh") * on
        return dict(on=on, own=own, fresh=fresh, flinch_ok=flinch_ability_ok * flinch_other_ok,
                    flinch_other_ok=flinch_other_ok, grounded=grounded, priority_ok=priority_ok, repeat=repeat,
                    can_protect=self._has_move(ids, floats, self.move_protect) * repeat * on,
                    fake_out_ready=self._has_move(ids, floats, self.move_fake_out) * fresh,
                    unseen_fist=self._ability_p(ids, floats, self.ability_unseen_fist))

    def effects(self, ids, field, t, p, moves, hit, faster_foe, own_eff, threat_from_foe, ko_from_foe, through,
                through_ko):
        """The ACTION_COLUMNS before the Tera ones: [B, 2, 4, 5, 10], the same with and without Tera."""
        B = ids.shape[0]
        own_slots, foe_tokens = list(OWN_TOKENS), list(FOE_TOKENS)
        user = lambda x: x[:, own_slots].view(B, 2, 1, 1)  # noqa: E731
        foe_on = p["on"][:, foe_tokens]  # [B, 2]
        on_target = (hit > 0).float()  # [B, 2, 4, 5, 12]
        share = torch.where(self.move_target[moves] == TARGETS.index("randomNormal"), 0.5, 1.0).view(B, 2, 4, 1, 1)
        mv = lambda table: table[moves].view(B, 2, 4, 1)  # noqa: E731
        ability = ids[:, own_slots, I_ABILITY].view(B, 2, 1)

        # Priority as used: Prankster status moves, Gale Wings at full HP, Grassy Glide in Grassy Terrain.
        prankster = (ability == self.ability["prankster"]) & (self.move_category[moves] == 0)  # [B, 2, 4]
        gale = ((ability == self.ability["galewings"]) & (self.move_type[moves] == self.flying)
                & (t["hp_frac"][:, own_slots] >= 1).view(B, 2, 1))
        glide = ((moves == self.grassy_glide) & (field[:, FF["grassyterrain"]] > 0.5).view(B, 1, 1)
                 & (p["grounded"][:, own_slots] > 0.5).view(B, 2, 1))
        priority = (self.move_priority[moves] + prankster.float() + gale.float() + glide.float()).view(B, 2, 4, 1)

        # Fails outright: first-turn-only moves after a move action; priority moves at a protected foe; Prankster
        # status moves at a Dark-type foe.
        stale = mv(self.move_first_only) * (1 - user(p["fresh"]))
        blocked = (on_target[..., 6:8] * (1 - p["priority_ok"][:, None, None, None, 6:8])).amax(-1)  # [B, 2, 4, 5]
        dark = (t["def_types"][:, foe_tokens] == self.dark).any(-1).float()  # [B, 2]
        prankster_dark = prankster.float().unsqueeze(-1) * (on_target[..., foe_tokens] * dark.view(B, 1, 1, 1, 2)).amax(-1)
        fail = 1 - (1 - stale) * (1 - (priority > 0).float() * blocked) * (1 - prankster_dark)

        # Acting before each foe: priority, then speed; a foe's first-turn Fake Out (+3) can come first.
        speed = faster_foe[:, own_slots].view(B, 2, 1, 2)  # [B, 2, 1, 2]
        before = torch.where(priority > 0, torch.ones_like(speed), torch.where(priority < 0, torch.zeros_like(speed), speed))
        race = torch.where(priority > 3, torch.zeros_like(speed), torch.where(priority == 3, 1 - speed, torch.ones_like(speed)))
        first = before * (1 - p["fake_out_ready"][:, foe_tokens].view(B, 1, 1, 2) * race)  # [B, 2, 4, 2]

        # Flinches before the foe acts (Sheer Force removes them, Serene Grace doubles them, Mold Breaker ignores
        # Inner Focus and Shield Dust), and the damage that stops.
        sheer = (ability == self.ability["sheerforce"]).float().unsqueeze(-1)  # [B, 2, 1, 1]
        serene = (ability == self.ability["serenegrace"]).float().unsqueeze(-1)
        chance = (mv(self.move_flinch) * (1 + serene)).clamp(max=1) * (1 - sheer)  # [B, 2, 4, 1]
        mold = self.ability_mold_breaker[ids[:, own_slots, I_ABILITY]].view(B, 2, 1)  # [B, 2, 1]
        with_ability, without = p["flinch_ok"][:, None, foe_tokens], p["flinch_other_ok"][:, None, foe_tokens]
        can_flinch = with_ability + (without - with_ability) * mold  # [B, 2 slots, 2 foes]
        can_flinch = (can_flinch * foe_on.unsqueeze(1)).view(B, 2, 1, 1, 2)  # [B, 2 slots, 1, 1, 2 foes]
        prio_ok = torch.where((priority > 0).unsqueeze(-1), p["priority_ok"][:, foe_tokens].view(B, 1, 1, 1, 2),
                              torch.ones(1, device=ids.device))  # [B, 2, 4, 1, 2]
        landed = (own_eff[..., foe_tokens] > 0).float().unsqueeze(3)  # [B, 2, 4, 1, 2]
        flinch = (chance.unsqueeze(-1) * mv(self.move_accuracy).unsqueeze(-1) * on_target[..., foe_tokens] * share
                  * first.unsqueeze(3) * can_flinch * prio_ok * landed * (1 - stale).unsqueeze(-1))  # [B, 2, 4, 5, 2]
        own_on = p["on"][:, own_slots].unsqueeze(-1)  # [B, 2, 1]
        foe_threat = (threat_from_foe[:, own_slots] * own_on).amax(1) * foe_on  # [B, 2 foes]
        foe_kos = (ko_from_foe[:, own_slots] * own_on).amax(1) * foe_on
        stop_damage = (flinch * foe_threat.view(B, 1, 1, 1, 2)).sum(-1)
        stop_ko = (flinch * foe_kos.view(B, 1, 1, 1, 2)).sum(-1)

        # Own Protect: success chance, and the damage and KO chance it blocks (not what gets through it).
        protect = mv(self.move_protect) * user(p["repeat"]) * user(p["on"])  # [B, 2, 4, 1]
        faced = lambda threat: (threat * foe_on.unsqueeze(1)).sum(-1).clamp(max=1)  # noqa: E731  [B, 2]
        faced_ko = lambda ko: 1 - torch.prod(1 - ko * foe_on.unsqueeze(1), -1)  # noqa: E731
        protect_damage = protect * (faced(threat_from_foe[:, own_slots]) - faced(through)).view(B, 2, 1, 1)
        protect_ko = protect * (faced_ko(ko_from_foe[:, own_slots]) - faced_ko(through_ko)).view(B, 2, 1, 1)

        # Foes that can Protect against the move.
        unseen = p["unseen_fist"][:, own_slots].view(B, 2, 1, 1) * mv(self.move_contact)
        blockable = mv(self.move_blockable) * (1 - unseen)
        foe_protect = on_target[..., foe_tokens] * p["can_protect"][:, foe_tokens].view(B, 1, 1, 1, 2) * blockable.unsqueeze(-1)

        expand = lambda x: x.expand(-1, -1, -1, 5)  # noqa: E731
        effects = torch.stack((
            fail, flinch[..., 0], flinch[..., 1], stop_damage, stop_ko,
            expand(protect), expand(protect_damage), expand(protect_ko), foe_protect[..., 0], foe_protect[..., 1],
        ), -1)
        return effects

    def tera_effects(self, floats, p, hit, foe_mask, own_dealt, own_ko, tera_dealt, tera_ko, threat_from_foe,
                     ko_from_foe, tera_threat, tera_threat_ko):
        """The Tera columns [B, 2, 4, 5, 4]: the change of damage and KO chance on the foes (Tera STAB) and of the
        damage and KO chance the user faces this turn (its Tera type) if the user terastallizes now."""
        B = floats.shape[0]
        own_slots = list(OWN_TOKENS)
        foe_on = p["on"][:, list(FOE_TOKENS)].unsqueeze(1)  # [B, 1, 2]
        faced = lambda threat: (threat * foe_on).sum(-1).clamp(max=1)  # noqa: E731
        faced_ko = lambda ko: 1 - torch.prod(1 - ko * foe_on, -1)  # noqa: E731
        can_tera = floats[:, own_slots, PF["tera_available"]] * (1 - floats[:, own_slots, PF["terastallized"]])
        defence = torch.stack((faced(tera_threat) - faced(threat_from_foe[:, own_slots]),
                               faced_ko(tera_threat_ko) - faced_ko(ko_from_foe[:, own_slots])), -1)  # [B, 2, 2]
        offence = torch.stack(((hit * (tera_dealt - own_dealt).unsqueeze(3) * foe_mask).sum(-1),
                               (hit * (tera_ko - own_ko).unsqueeze(3) * foe_mask).sum(-1)), -1)  # [B, 2, 4, 5, 2]
        tera = torch.cat((offence, defence.view(B, 2, 1, 1, 2).expand(-1, -1, 4, 5, -1)), -1)
        return tera * can_tera.view(B, 2, 1, 1, 1)

    def token_effects(self, ids, floats, t, p) -> torch.Tensor:
        """The TOKEN_COLUMNS: [B, 12, 4]."""
        ready = p["fake_out_ready"]
        n = self.n_types
        flat = self.typechart.reshape(-1)
        normal = (flat[self.normal * n + t["def_types"][..., 0]] * flat[self.normal * n + t["def_types"][..., 1]]
                  * t["type_mult"][..., self.normal])
        flinchable = p["flinch_ok"] * p["priority_ok"] * (normal > 0).float() * t["alive"]
        from_own = 1 - torch.prod(1 - ready[:, list(OWN_TOKENS)], -1, keepdim=True)
        from_foe = 1 - torch.prod(1 - ready[:, list(FOE_TOKENS)], -1, keepdim=True)
        threat = flinchable * p["on"] * torch.where(p["own"], from_foe, from_own)
        return torch.stack((ready, flinchable, threat, p["can_protect"]), -1) * floats[..., PF["present"]].unsqueeze(-1)
