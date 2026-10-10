"""A doubles-aware scripted player ("smart"), used as an evaluation baseline and as a scripted training opponent.

It has the interface of the agents in ``evaluate.py``: ``first(arrays, rows, mask0) -> a0``, ``second(rows, mask1)
-> a1`` and ``name``. It decides only from the row's own observation arrays (``ids``, ``floats``, ``field``,
``action_features``, ``token_features``, all built from the player's own request and player-view log) and the
legality masks, so it sees nothing the policy does not.

Every legal code of both slots gets an effect estimate from the damage-calc module (training.damage): expected damage
and KO chance on each foe and on the partner, whether it acts before each foe, Fake Out flinches, Protect, Helping Hand
and redirection, the defensive change of Tera, and the incoming damage and matchup of a switch target. Pairs of slot
a and slot b actions are scored jointly, which gives:

* focus fire without overkill: combined damage on a foe is capped at its HP, so when one attack likely KOs a foe the
  partner gains more on the other foe; two attacks that only KO together are valued as a likely KO,
* KOs before a foe moves (and Fake Out flinches) remove that foe's damage this turn,
* Protect when the protected Pokemon is threatened, never twice in a row, skipped at random and with a jittered value,
* Tera only when it turns a non-KO into a KO or prevents a likely KO (and then still has to pay a cost),
* voluntary switches only when the active is badly threatened and a bench Pokemon takes much less damage,
* forced replacements by the best expected matchup,
* damage to the partner weighted 1.5x, so spread moves hit the ally only for a clear net gain,
* Fake Out on a Pokemon's first turn out, when legal and the target can flinch, unless an attack likely KOs.

Common status moves get small fixed values where they do something (Tailwind, Trick Room, sleep, burn, paralysis,
setup and recovery when safe, screens, hazards); other status moves are only used when nothing else is worth more.
The foes' targets are guessed from the threat features (a foe more likely attacks where it does more damage), and a
foe that has revealed Protect (or likely has it, per the set belief) may protect, which blocks both of our attacks at
once. Slot b's mask depends on slot a's choice, so ``first`` plans the pair with an estimate of slot b's options and
``second`` re-picks slot b over its exact mask given slot a's code.
"""
from __future__ import annotations

import numpy as np
import torch

from .damage import SPREAD, DamageFeatures, _hit_table, any_ko, expected_max
from .dex import TARGETS, Dex, load_dex
from .encoder import (FF, I_ABILITY, I_CAND_ABILITIES, I_CAND_MOVES, I_ITEM, I_MOVE0, I_TERA, I_TYPE1, K_ABILITIES,
                      K_MOVES, PF)

N_ACTIONS = 47
PASS = 46
OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")

# Values are in fractions of a Pokemon's max HP.
KO_VALUE = 0.35  # a foe KO beyond the damage, times (1 + how dangerous that foe is)
LOSS_VALUE = 0.35  # losing one of ours beyond its HP
ALLY_WEIGHT = 1.5  # damage to the partner counts 1.5x
FLINCH_VALUE = 0.3  # per foe flinched, on top of the damage it would have dealt
TERA_COST = 0.2  # the option value of keeping Tera
TERA_GATE = 0.25  # Tera is considered only for a KO-chance swing at least this large
SWITCH_COST = 0.12
MATCHUP_WEIGHT = 0.5
THREATENED = 0.5  # KO chance from a foe that makes a voluntary switch worth considering
PROTECT_SHARE = (0.55, 0.85)  # share of the avoided damage credited to Protect (it only buys one turn), per decision
PROTECT_SKIP = 0.15  # chance per slot and decision to not consider Protect at all
FAKE_OUT_KO = 0.8  # an attack with this KO chance may replace a first-turn Fake Out
NOISE = 0.005

PROTECT_MOVES = ("protect", "detect", "spikyshield", "kingsshield", "banefulbunker", "obstruct", "silktrap",
                 "burningbulwark")
FIRST_TURN_ONLY = ("fakeout", "firstimpression")
FLINCH_BLOCK = ("innerfocus", "shielddust")
PRIORITY_BLOCK = ("armortail", "dazzling", "queenlymajesty")
SLEEP_MOVES = ("spore", "sleeppowder", "hypnosis", "sing", "lovelykiss", "grasswhistle", "darkvoid", "yawn")
SLEEP_BLOCK = ("insomnia", "vitalspirit", "sweetveil", "comatose", "purifyingsalt")
OFFENSIVE_SETUP = ("swordsdance", "nastyplot", "calmmind", "dragondance", "quiverdance", "shellsmash", "bulkup", "coil",
                   "tailglow", "noretreat", "tidyup", "victorydance", "shiftgear", "workup", "growth", "filletaway",
                   "clangoroussoul", "geomancy", "honeclaws", "rockpolish", "agility", "curse", "bellydrum")
DEFENSIVE_SETUP = ("irondefense", "cosmicpower", "amnesia", "acidarmor", "cottonguard", "stockpile")
RECOVERY = ("recover", "roost", "slackoff", "synthesis", "moonlight", "morningsun", "softboiled", "shoreup",
            "milkdrink", "healorder", "strengthsap")
TEAM_HEAL = ("lifedew", "lunarblessing", "junglehealing")
ALLY_HEAL = ("healpulse", "floralhealing", "pollenpuff")
HAZARDS = {"stealthrock": 0.05, "spikes": 0.035, "toxicspikes": 0.025, "stickyweb": 0.04}
SCREENS = ("reflect", "lightscreen", "auroraveil")


def _codes_of(mask: np.ndarray) -> np.ndarray:
    return np.flatnonzero(mask)


class SmartHeuristicAgent:
    """Doubles-aware scripted baseline; see the module docstring.

    ``first`` keeps per-row state for the following ``second`` call on the same rows. ``temperature`` > 0 adds Gumbel
    noise of that scale to the pair values (values are in fractions of max HP), e.g. 0.05 to make it less predictable
    as a training opponent; 0 picks the best pair (ties broken at random)."""

    name = "smart"

    def __init__(self, rng: np.random.Generator, dex: Dex | None = None, temperature: float = 0.0):
        self.rng = rng
        self.temperature = temperature
        self.dex = dex or load_dex()
        self.damage = DamageFeatures(self.dex)
        dex = self.dex
        n = dex.n_moves

        def lookup(names):
            table = np.zeros(n, dtype=bool)
            for name in names:
                if name in dex.move_index:
                    table[dex.move_index[name]] = True
            return table

        self.is_protect = lookup(PROTECT_MOVES)
        self.is_first_turn_only = lookup(FIRST_TURN_ONLY)
        self.is_fake_out = lookup(("fakeout",))
        self.is_helping_hand = lookup(("helpinghand",))
        self.is_redirect = lookup(("followme", "ragepowder"))
        self.recoil = np.zeros(n, dtype=np.float32)
        self.self_destruct = np.zeros(n, dtype=bool)
        self.recharge = np.zeros(n, dtype=bool)
        self.harmful_status = np.zeros(n, dtype=bool)
        self.status_kind = [""] * n
        for i, move in enumerate(dex.moves, start=1):
            self.recoil[i] = float(move.get("recoil") or 0)
            self.self_destruct[i] = bool(move.get("selfdestruct"))
            self.recharge[i] = "recharge" in (move.get("flags") or ())
            if move["category"] == "Status":
                self.status_kind[i] = self._status_kind(move["id"])
                # Targeted status is hostile unless its effects or target class identify ally support.
                boosts = move.get("boosts") or {}
                harmful_effect = (bool(move.get("status")) or bool(move.get("forceSwitch"))
                                  or any(v < 0 for v in boosts.values())
                                  or move.get("volatileStatus") in ("confusion", "taunt", "encore", "disable",
                                                                    "yawn", "leechseed", "curse"))
                support = (move["id"] in ALLY_HEAL or "allyanim" in (move.get("flags") or ())
                           or bool(boosts) and all(v >= 0 for v in boosts.values()))
                self.harmful_status[i] = (move["target"] in ("normal", "any", "adjacentFoe")
                                          and (harmful_effect or not support))
            elif move["id"] == "pollenpuff":
                self.status_kind[i] = "allyheal"
        self.has_status_value = np.array([bool(k) and k != "protect" for k in self.status_kind]) | self.harmful_status
        self.ability = {name: dex.ability_index.get(name, -1) for name in
                        FLINCH_BLOCK + PRIORITY_BLOCK + SLEEP_BLOCK + ("levitate",)}
        self.covert_cloak = dex.item_index.get("covertcloak", -1)
        self.air_balloon = dex.item_index.get("airballoon", -1)
        self.type = {name: dex.type_idx(name) for name in ("Electric", "Ground", "Fire", "Grass", "Poison", "Steel", "Flying")}
        self.hit = _hit_table()
        self.random_normal = TARGETS.index("randomNormal")
        self.spread_targets = (TARGETS.index("allAdjacentFoes"), TARGETS.index("allAdjacent"))
        self.ctx: dict = {}

    @staticmethod
    def _status_kind(move_id: str) -> str:
        for kind, names in (("protect", PROTECT_MOVES), ("setup", OFFENSIVE_SETUP), ("defense", DEFENSIVE_SETUP),
                            ("recovery", RECOVERY), ("teamheal", TEAM_HEAL), ("allyheal", ALLY_HEAL),
                            ("screen", SCREENS)):
            if move_id in names:
                return kind
        if move_id in SLEEP_MOVES:
            return "sleep"
        if move_id in HAZARDS:
            return "hazard"
        return {"tailwind": "tailwind", "trickroom": "trickroom", "thunderwave": "paralysis", "glare": "paralysis",
                "willowisp": "burn", "toxic": "poison", "leechseed": "leechseed", "revivalblessing": "revive",
                "rest": "rest"}.get(move_id, "")

    # ---- damage tables ------------------------------------------------------------------------------------------
    def _tables(self, ids: np.ndarray, floats: np.ndarray, field: np.ndarray):
        """For own tokens 0-5 as attackers: expected dealt fraction and KO chance of each move slot on every token
        ([B, 6, 4, 12], spread moves with their 0.75 modifier), the move target classes, and the threat and KO chance
        from each foe active on every token ([B, 12, 2])."""
        dm = self.damage
        with torch.no_grad():
            ids_t = torch.from_numpy(np.ascontiguousarray(ids))
            floats_t = torch.from_numpy(np.ascontiguousarray(floats))
            field_t = torch.from_numpy(np.ascontiguousarray(field))
            B = ids_t.shape[0]
            t = dm.tokens(ids_t, floats_t, field_t)
            defenders = {k: v.reshape(B, 1, 1, 12, *v.shape[2:]) for k, v in t.items()}
            own = {k: v[:, :6].reshape(B, 6, 1, 1, *v.shape[2:]) for k, v in t.items()}
            moves = ids_t[:, :6, I_MOVE0:I_MOVE0 + 4]
            target = dm.move_target[moves]
            spread = (target == self.spread_targets[0]) | (target == self.spread_targets[1])
            mv = lambda table: table[moves].unsqueeze(-1)
            dealt, ko, _ = dm.damage(own, defenders, mv(dm.move_type), mv(dm.move_category), mv(dm.move_power),
                                     mv(dm.move_accuracy), mv(dm.move_hits), mv(dm.move_special), field_t,
                                     modifier=torch.where(spread, SPREAD, 1.0).unsqueeze(-1))
            foe_dealt, foe_ko, foe_probs = dm.foe_attacks(ids_t, floats_t, t, defenders, field_t)
            threat = expected_max(foe_dealt, foe_probs).transpose(1, 2)
            kothreat = any_ko(foe_ko, foe_probs).transpose(1, 2)
        return dealt.numpy(), ko.numpy(), target.numpy(), threat.numpy(), kothreat.numpy()

    def _per_code(self, dealt: np.ndarray, ko: np.ndarray, target: np.ndarray, slot: int):
        """Split the moves of active ``slot`` over the 20 (move slot, target code) pairs: [B, 20, 12] each."""
        hit = self.hit[target[:, slot]][:, :, slot]  # [B, 4, 5, 12]
        share = (hit > 0) * np.where(target[:, slot] == self.random_normal, 0.5, 1.0)[:, :, None, None]
        B = len(dealt)
        d = (share * dealt[:, slot, :, None, :]).reshape(B, 20, 12)
        k = (share * ko[:, slot, :, None, :]).reshape(B, 20, 12)
        return d.astype(np.float32), k.astype(np.float32)

    # ---- per-decision context -----------------------------------------------------------------------------------
    def _prepare(self, obs: dict) -> dict:
        ids, floats, field, af, tf = (obs[k] for k in OBS_KEYS)
        B = len(ids)
        P = lambda name: floats[..., PF[name]]
        hp = P("hp")
        alive = (P("present") > 0.5) & (P("fainted") < 0.5)
        on = alive & ((P("active_a") + P("active_b")) > 0.5)
        own_on = on[:, :2].astype(np.float32)
        foe_on = on[:, 6:8].astype(np.float32)
        threat, kothreat, faster = tf[..., 0:2], tf[..., 2:4], tf[..., 4:6]  # [B, 12, 2]

        dealt, ko, target, _, _ = self._tables(ids, floats, field)
        split = [self._per_code(dealt, ko, target, s) for s in (0, 1)]
        code_d = np.stack([d for d, _ in split], 1)  # [B, 2, 20, 12]
        code_k = np.stack([k for _, k in split], 1)
        code_d_t, code_k_t = code_d.copy(), code_k.copy()
        threat_t, kothreat_t = threat[:, :2].copy(), kothreat[:, :2].copy()  # [B, 2 slots, 2 foes]
        tera_usable = on[:, :2] & (P("tera_available")[:, :2] > 0.5) & (field[:, FF["own_tera_used"]] < 0.5)[:, None]
        for s in (0, 1):
            rows = np.flatnonzero(tera_usable[:, s])
            if not len(rows):
                continue
            floats_t = floats[rows].copy()
            floats_t[:, s, PF["terastallized"]] = 1.0
            d2, k2, tgt2, th2, ko2 = self._tables(ids[rows], floats_t, field[rows])
            code_d_t[rows, s], code_k_t[rows, s] = self._per_code(d2, k2, tgt2, s)
            threat_t[rows, s], kothreat_t[rows, s] = th2[:, s], ko2[:, s]

        # Foe targeting: a foe more likely attacks the active it hurts more.
        q = np.zeros((B, 2, 2), dtype=np.float32)  # [B, foe, own slot]
        for f in (0, 1):
            t0, t1 = threat[:, 0, f] * own_on[:, 0], threat[:, 1, f] * own_on[:, 1]
            share = np.clip((t0 + 0.05) / (t0 + t1 + 0.1), 0.15, 0.85)
            both = (own_on[:, 0] > 0) & (own_on[:, 1] > 0)
            q[:, f, 0] = np.where(both, share, own_on[:, 0])
            q[:, f, 1] = np.where(both, 1 - share, own_on[:, 1])
        # Chance that a foe protects: it has revealed Protect (or likely has it), did not protect last turn, and is
        # more likely to when we threaten a KO.
        revealed = self.is_protect[ids[:, 6:8, I_MOVE0:I_MOVE0 + 4]].any(-1)
        cand = (floats[:, 6:8, PF["cand_move_p0"]:PF["cand_move_p0"] + K_MOVES]
                * self.is_protect[ids[:, 6:8, I_CAND_MOVES:I_CAND_MOVES + K_MOVES]]).sum(-1)
        has = np.where(revealed, 1.0, np.minimum(cand, 1.0))
        danger = (tf[:, 6:8, 2:4] * own_on[:, None, :]).max(-1)
        foe_protect = foe_on * has * (1 - P("protect_last")[:, 6:8]) * (0.12 + 0.3 * danger)
        foe_value = ((threat[:, :2] + 0.5 * kothreat[:, :2]) * own_on[:, :, None]).max(1)  # [B, foe]

        # Matchup of every own Pokemon against the foe actives (for switching and replacements).
        off = dealt[:, :, :, 6:8].max(2)  # [B, 6, 2]
        off_ko = ko[:, :, :, 6:8].max(2)
        terms = off + 0.5 * off_ko - threat[:, :6] - 0.5 * kothreat[:, :6] + 0.2 * (faster[:, :6] - 0.5)
        n_foes = foe_on.sum(-1, keepdims=True)
        matchup = (terms * foe_on[:, None, :]).sum(-1) / np.maximum(n_foes, 1) + 0.1 * hp[:, :6]
        matchup = np.where(alive[:, :6], matchup, -np.inf)

        # Effects of every code of both slots.
        codes = np.arange(40)
        m, tc, tbit = codes // 10, (codes % 10) // 2, codes % 2
        k = m * 5 + tc
        tera_code = (tbit == 1)[None, None, :, None]
        D = np.where(tera_code, code_d_t[:, :, k], code_d[:, :, k])  # [B, 2, 40, 12]
        K = np.where(tera_code, code_k_t[:, :, k], code_k[:, :, k])
        mid = np.stack([ids[:, s, I_MOVE0 + m] for s in (0, 1)], 1)  # [B, 2, 40] move id of each code

        shape = (B, 2, N_ACTIONS)
        e = dict(
            off_d=np.zeros(shape + (2,), np.float32), off_k=np.zeros(shape + (2,), np.float32),
            first=np.zeros(shape + (2,), np.float32), flinch=np.zeros(shape + (2,), np.float32),
            inc_d=np.zeros(shape + (2,), np.float32), inc_k=np.zeros(shape + (2,), np.float32),
            ally_d=np.zeros(shape, np.float32), ally_k=np.zeros(shape, np.float32),
            protect=np.zeros(shape, np.float32), helping=np.zeros(shape, np.float32),
            redirect=np.zeros(shape, np.float32), bonus=np.zeros(shape, np.float32),
            act_bonus=np.zeros(shape, np.float32), tera=np.zeros(shape, bool), switch=np.full(shape, -1),
            allowed=np.ones(shape, bool), fake_out=np.zeros(shape, bool),
        )
        e["off_d"][:, :, :40] = D[..., 6:8]
        e["off_k"][:, :, :40] = K[..., 6:8]
        e["ally_d"][:, 0, :40], e["ally_d"][:, 1, :40] = D[:, 0, :, 1], D[:, 1, :, 0]
        e["ally_k"][:, 0, :40], e["ally_k"][:, 1, :40] = K[:, 0, :, 1], K[:, 1, :, 0]
        e["first"][:, :, :40] = af[..., 6:8]
        e["inc_d"][:, :, :40] = np.where(tera_code, threat_t[:, :, None], threat[:, :2, None])
        e["inc_k"][:, :, :40] = np.where(tera_code, kothreat_t[:, :, None], kothreat[:, :2, None])
        e["inc_d"][:, :, PASS], e["inc_k"][:, :, PASS] = threat[:, :2], kothreat[:, :2]
        e["inc_d"][:, :, 40:46] = threat[:, None, :6]
        e["inc_k"][:, :, 40:46] = kothreat[:, None, :6]
        e["switch"][:, :, 40:46] = np.arange(6)
        e["protect"][:, :, :40] = self.is_protect[mid]
        e["helping"][:, :, :40] = self.is_helping_hand[mid]
        e["redirect"][:, :, :40] = self.is_redirect[mid]
        e["tera"][:, :, :40] = tbit == 1
        e["bonus"][:, :, :40] -= TERA_COST * (tbit == 1)
        sum_d = e["off_d"][:, :, :40].sum(-1)
        e["act_bonus"][:, :, :40] -= 0.25 * self.recoil[mid] * sum_d + 0.2 * self.recharge[mid]
        e["act_bonus"][:, :, :40] -= self.self_destruct[mid] * (hp[:, :2, None] + LOSS_VALUE)

        # First-turn-only moves fail later; Fake Out flinches a foe that can flinch.
        fresh = (P("fresh")[:, :2] > 0.5) & (P("switch_turns")[:, :2] < 0.3) & on[:, :2]  # [B, 2]
        first_only = self.is_first_turn_only[mid]
        stale = first_only & ~fresh[:, :, None]
        e["off_d"][:, :, :40][stale] = 0
        e["off_k"][:, :, :40][stale] = 0
        can_flinch = self._can_flinch(ids, floats, field) * foe_on  # [B, foe]
        immune = af[..., 5] > 0.5  # [B, 2, 40]
        fake = self.is_fake_out[mid] & fresh[:, :, None]
        for f in (0, 1):
            hits = fake & (tc == 3 + f) & ~immune
            e["flinch"][:, :, :40, f] = hits * e["first"][:, :, :40, f] * can_flinch[:, f, None, None]
        e["fake_out"][:, :, :40] = e["flinch"][:, :, :40].max(-1) > 0.05

        # Tera only for a KO swing on a foe, or to prevent a likely KO of the user.
        ko_gain = (code_k_t[:, :, k][..., 6:8] - code_k[:, :, k][..., 6:8]).max(-1)  # [B, 2, 40]
        own_risk = (kothreat[:, :2] * foe_on[:, None, :]).max(-1)  # [B, 2]
        tera_risk = (kothreat_t * foe_on[:, None, :]).max(-1)
        saves = (own_risk >= 0.4) & (own_risk - tera_risk >= TERA_GATE)
        tera_ok = (ko_gain >= TERA_GATE) | saves[:, :, None]
        e["allowed"][:, :, :40] &= (tbit == 0) | tera_ok

        # Voluntary switches only out of a bad spot into a much safer Pokemon.
        request_move = field[:, FF["request_move"]] > 0.5
        threatened = own_risk >= THREATENED
        exposure = (threat[:, :6] * foe_on[:, None, :]).sum(-1)  # [B, 6]
        bench_risk = (kothreat[:, :6] * foe_on[:, None, :]).max(-1)
        for s in (0, 1):
            safer = (exposure <= 0.5 * exposure[:, s:s + 1]) & (bench_risk <= 0.25)  # [B, 6]
            ok = np.where(request_move[:, None], threatened[:, s:s + 1] & safer, True)
            e["allowed"][:, s, 40:46] &= ok
            delta = matchup - np.where(np.isfinite(matchup[:, s:s + 1]), matchup[:, s:s + 1], 0)
            e["bonus"][:, s, 40:46] = np.where(request_move[:, None], MATCHUP_WEIGHT * delta - SWITCH_COST, 0)
        e["bonus"] = np.where(np.isfinite(e["bonus"]), e["bonus"], -10.0).astype(np.float32)

        # Protect: never twice in a row, sometimes not considered, credited with a jittered share.
        protect_last = P("protect_last")[:, :2] > 0.5
        skip = self.rng.random((B, 2)) < PROTECT_SKIP
        e["allowed"] &= ~((e["protect"] > 0) & (protect_last | skip)[:, :, None])

        ctx = dict(B=B, hp=hp, alive=alive, on=on, own_on=own_on, foe_on=foe_on, q=q, foe_protect=foe_protect,
                   foe_value=foe_value, matchup=matchup, request_move=request_move,
                   protect_share=self.rng.uniform(*PROTECT_SHARE, size=B).astype(np.float32),
                   ids=ids, floats=floats, field=field, faster=faster, threat=threat, kothreat=kothreat,
                   tera_usable=tera_usable, af=af)
        self._status_values(ctx, e)
        ctx["strong"] = (e["off_k"] * foe_on[:, None, None, :]).max(-1) >= FAKE_OUT_KO  # [B, 2, 47]
        ctx["e"] = e
        return ctx

    def _can_flinch(self, ids, floats, field) -> np.ndarray:
        """[B, foe]: chance that a Fake Out on the foe active flinches it (known and believed abilities, Covert
        Cloak, Substitute, Psychic Terrain and priority-blocking abilities on the foe side)."""
        B = len(ids)
        out = np.ones((B, 2), dtype=np.float32)
        ability = ids[:, 6:8, I_ABILITY]
        cand_ids = ids[:, 6:8, I_CAND_ABILITIES:I_CAND_ABILITIES + K_ABILITIES]
        cand_p = floats[:, 6:8, PF["cand_ability_p0"]:PF["cand_ability_p0"] + K_ABILITIES]
        block = np.isin(ability, [self.ability[a] for a in FLINCH_BLOCK])
        believed = (cand_p * np.isin(cand_ids, [self.ability[a] for a in FLINCH_BLOCK])).sum(-1)
        out *= np.where(ability > 0, 1.0 - block, 1.0 - np.minimum(believed, 1.0))
        out *= ids[:, 6:8, I_ITEM] != self.covert_cloak
        out *= floats[:, 6:8, PF["substitute"]] < 0.5
        side_block = np.isin(ability, [self.ability[a] for a in PRIORITY_BLOCK]).any(-1)
        out *= ~side_block[:, None]
        flying = np.stack([(self._types(ids, floats, 6 + f) == self.type["Flying"]).any(-1) for f in (0, 1)], -1)
        levitate = ability == self.ability["levitate"]
        believed_levitate = (cand_p * (cand_ids == self.ability["levitate"])).sum(-1)
        airborne = np.where(ability > 0, levitate, np.clip(believed_levitate, 0, 1))
        airborne = np.where(flying | (ids[:, 6:8, I_ITEM] == self.air_balloon), 1.0, airborne)
        airborne = np.where((field[:, FF["gravity"]] > 0.5)[:, None], 0.0, airborne)
        out *= np.where((field[:, FF["psychicterrain"]] > 0.5)[:, None], airborne, 1.0)
        return out

    def _types(self, ids, floats, token):
        """Current types of ``token`` per row ([B, 2]; the Tera type alone once terastallized)."""
        tera = ids[:, token, I_TERA]
        tera_on = (floats[:, token, PF["terastallized"]] > 0.5) & (tera > 0) & (tera != 19)
        return np.where(tera_on[:, None], np.stack((tera, np.zeros_like(tera)), -1), ids[:, token, I_TYPE1:I_TYPE1 + 2])

    def _status_values(self, c: dict, e: dict) -> None:
        """Small fixed values for common status moves (``act_bonus``), only where the move does something."""
        ids, floats, field, faster = c["ids"], c["floats"], c["field"], c["faster"]
        own_on, foe_on, hp, kothreat, q = c["own_on"], c["foe_on"], c["hp"], c["kothreat"], c["q"]
        P = lambda name: floats[..., PF[name]]
        statused = np.stack([P(s) for s in ("brn", "par", "slp", "frz", "psn", "tox")], -1).max(-1) > 0.5  # [B, 12]
        foe_asleep = (P("slp")[:, 6:12] > 0.5).any(-1)
        trick_room = field[:, FF["trickroom"]] > 0.5
        foe_types = [self._types(ids, floats, 6 + f) for f in (0, 1)]
        T = self.type
        abilities = ids[:, 6:8, I_ABILITY]
        sleep_block = np.isin(abilities, [self.ability[a] for a in SLEEP_BLOCK])
        grounded_block = (field[:, FF["electricterrain"]] > 0.5) | (field[:, FF["mistyterrain"]] > 0.5)
        immune = c["af"][..., 5] > 0.5
        dex = self.dex
        for b, s, m in zip(*np.nonzero(self.has_status_value[ids[:, :2, I_MOVE0:I_MOVE0 + 4]])):
            move = ids[b, s, I_MOVE0 + m]
            kind = self.status_kind[move]
            if not own_on[b, s]:
                continue
            name = dex.moves[move - 1]["id"]
            accuracy = float(dex.move_accuracy[move])
            risk = float((q[b, :, s] * foe_on[b] * kothreat[b, s]).sum())
            safe = risk < 0.2 and hp[b, s] >= 0.6
            value, per_foe = 0.0, None
            slower = sum((1 - faster[b, s2, f]) * own_on[b, s2] * foe_on[b, f] for s2 in (0, 1) for f in (0, 1))
            quicker = sum(faster[b, s2, f] * own_on[b, s2] * foe_on[b, f] for s2 in (0, 1) for f in (0, 1))
            if kind == "tailwind":
                value = 0.0 if field[b, FF["own_tailwind"]] > 0.5 or trick_room[b] else 0.06 + 0.1 * slower
            elif kind == "trickroom":
                value = max(0.0, 0.1 * (slower - quicker))
            elif kind == "setup":
                boost = max(P("boost_atk")[b, s], P("boost_spa")[b, s])
                value = (0.3 if name in ("shellsmash", "bellydrum") else 0.22) if safe and boost < 0.3 else 0.0
                if name == "bellydrum" and hp[b, s] < 0.75:
                    value = -0.5
            elif kind == "defense":
                value = 0.08 if safe else 0.0
            elif kind == "recovery":
                missing = min(0.5, 1.0 - hp[b, s])
                value = (0.8 if hp[b, s] < 0.6 else 0.2) * missing
            elif kind == "rest":
                value = 0.35 if hp[b, s] < 0.4 and not statused[b, s] else 0.0
            elif kind == "teamheal":
                value = 0.8 * sum(min(0.25, 1.0 - hp[b, t]) * own_on[b, t] for t in (0, 1))
            elif kind == "screen":
                active = field[b, FF["own_" + name]] > 0.5
                needs_snow = name == "auroraveil" and field[b, FF["snow"]] < 0.5
                value = 0.0 if active or needs_snow else (0.25 if name == "auroraveil" else 0.15)
            elif kind == "hazard":
                bench = max(0.0, 6 - 6 * field[b, FF["foe_fainted"]] - foe_on[b].sum())
                full = field[b, FF["foe_" + name]] >= 0.99  # spikes count layers / 3, toxic spikes / 2
                value = 0.0 if full else HAZARDS[name] * bench
            elif kind == "revive":
                value = 0.4 if field[b, FF["own_alive"]] < 0.99 else -0.5
            elif kind == "allyheal":
                pass  # Value depends on the selected own token, below.
            else:  # foe-targeted status
                per_foe = np.zeros(2, dtype=np.float32)
                for f in (0, 1):
                    token = 6 + f
                    if not foe_on[b, f] or statused[b, token]:
                        continue
                    types = foe_types[f][b]
                    if kind == "paralysis":
                        if T["Electric"] in types:
                            continue
                        foe_faster = sum((1 - faster[b, s2, f]) * own_on[b, s2] for s2 in (0, 1))
                        per_foe[f] = accuracy * (0.12 + 0.12 * foe_faster)
                    elif kind == "burn":
                        if T["Fire"] in types:
                            continue
                        physical = P("stat_atk")[b, token] >= P("stat_spa")[b, token]
                        per_foe[f] = accuracy * (0.3 if physical else 0.05)
                    elif kind == "poison":
                        if T["Poison"] in types or T["Steel"] in types:
                            continue
                        per_foe[f] = accuracy * 0.15
                    elif kind == "sleep":
                        powder = name in ("spore", "sleeppowder")
                        if foe_asleep[b] or sleep_block[b, f] or grounded_block[b] or (powder and T["Grass"] in types):
                            continue
                        per_foe[f] = accuracy * (0.25 if name == "yawn" else 0.45)
                    elif kind == "leechseed":
                        if T["Grass"] in types or P("leechseed")[b, token] > 0.5:
                            continue
                        per_foe[f] = accuracy * 0.1
            for code in range(m * 10, m * 10 + 10):
                tcode = (code % 10) // 2
                if tcode in (0, 1) and self.harmful_status[move]:
                    token = 1 - tcode  # code 0 = own b; code 1 = own a
                    v = -ALLY_WEIGHT * own_on[b, token]
                elif kind == "allyheal":
                    v = 0.0
                    if tcode in (0, 1):
                        token = 1 - tcode
                        if own_on[b, token] and hp[b, token] < 0.5:
                            v = 0.8 * min(0.5, 1 - hp[b, token])
                        if name == "pollenpuff":
                            e["ally_d"][b, s, code] = e["ally_k"][b, s, code] = 0
                    elif name != "pollenpuff" and tcode in (3, 4):
                        v = -0.8 * min(0.5, 1 - hp[b, 6 + tcode - 3]) * foe_on[b, tcode - 3]
                elif per_foe is None:
                    v = value
                elif tcode in (3, 4):
                    v = 0.0 if immune[b, s, code] else float(per_foe[tcode - 3])
                elif tcode == 2:
                    v = float(per_foe.max())  # spread status moves
                else:
                    v = 0.0
                e["act_bonus"][b, s, code] += v

    # ---- candidates and pair values -----------------------------------------------------------------------------
    def _estimate_codes(self, c: dict, i: int, s: int) -> np.ndarray:
        """Codes slot ``s`` can likely take in a move request (used to plan slot a before slot b's mask exists)."""
        if not c["on"][i, s]:
            return np.array([PASS])
        ids, floats = c["ids"][i], c["floats"][i]
        dex = self.dex
        ally = 0 if s == 0 else 1
        tera = bool(c["tera_usable"][i, s])
        codes = []
        for m in range(4):
            move = ids[s, I_MOVE0 + m]
            if not move or floats[s, PF[f"pp{m}"]] <= 0 or floats[s, PF[f"disabled{m}"]] > 0.5:
                continue
            target = TARGETS[dex.move_target[move]]
            if target in ("normal", "any"):
                targets = (ally, 3, 4)
            elif target == "adjacentFoe":
                targets = (3, 4)
            elif target == "adjacentAlly":
                targets = (ally,)
            elif target == "adjacentAllyOrSelf":
                targets = (0, 1)
            else:
                targets = (2,)
            for t in targets:
                codes.append(m * 10 + t * 2)
                if tera:
                    codes.append(m * 10 + t * 2 + 1)
        if floats[s, PF["trapped"]] < 0.5:
            for p in range(2, 6):
                if c["alive"][i, p] and not c["on"][i, p]:
                    codes.append(40 + p)
        return np.array(codes or [PASS])

    def _candidates(self, c: dict, i: int, s: int, codes: np.ndarray) -> dict:
        e = c["e"]
        allowed = e["allowed"][i, s, codes]
        # First turn out with a working Fake Out: use it, unless another attack likely KOs a foe.
        fake = e["fake_out"][i, s, codes] & allowed
        if fake.any() and not (c["strong"][i, s, codes] & allowed & ~fake).any():
            allowed = fake
        if allowed.any():
            codes = codes[allowed]
        out = {k: v[i, s, codes] for k, v in e.items() if k not in ("allowed", "fake_out")}
        out["code"] = codes
        return out

    def _pair_values(self, c: dict, i: int, X: dict, Y: dict) -> np.ndarray:
        """Joint value [Na, Nb] of slot a candidates ``X`` and slot b candidates ``Y``."""
        foe_on, pp, q = c["foe_on"][i], c["foe_protect"][i], c["q"][i]
        hp_f = np.maximum(c["hp"][i, 6:8], 0.01)
        own_on = c["own_on"][i]
        share = c["protect_share"][i]
        x = lambda key: X[key][:, None]  # [Na, 1, ...]
        y = lambda key: Y[key][None, :]  # [1, Nb, ...]
        # A foe's Protect blocks both of our attacks at once, so everything below is conditioned on the foe not
        # protecting, and the foe's chance to protect is applied once per foe.
        # Foes stopped before acting: flinched, or KO'd by a faster attack.
        flinch = 1 - (1 - x("flinch")) * (1 - y("flinch"))  # [Na, Nb, 2]
        first_ko = 1 - (1 - x("off_k") * x("first")) * (1 - y("off_k") * y("first"))
        attacks = foe_on * (1 - pp) * (1 - flinch) * (1 - first_ko)  # foe f attacks us this turn
        # Where the foes aim: redirection pulls single-target attacks to the redirector.
        rx, ry = x("redirect")[..., None], y("redirect")[..., None]
        qa = np.where(rx > 0, 1.0, np.where(ry > 0, 0.0, q[:, 0]))
        qb = np.where(ry > 0, 1.0, np.where(rx > 0, 0.0, q[:, 1]))
        qa, qb = qa * own_on[0], qb * own_on[1]
        px, py = x("protect")[..., None], y("protect")[..., None]
        ax, ay = attacks * qa, attacks * qb
        keep_x, keep_y = 1 - share * px, 1 - share * py
        damage_a = (ax * x("inc_d") * keep_x).sum(-1)
        damage_b = (ay * y("inc_d") * keep_y).sum(-1)
        loss_a = 1 - np.prod(1 - ax * x("inc_k") * keep_x, -1)
        loss_b = 1 - np.prod(1 - ay * y("inc_k") * keep_y, -1)
        # KO'd before acting: our attack is lost.
        land_a = np.prod(1 - ax * x("inc_k") * (1 - x("first")) * (1 - px), -1)[..., None]
        land_b = np.prod(1 - ay * y("inc_k") * (1 - y("first")) * (1 - py), -1)[..., None]
        hx, hy = x("helping")[..., None], y("helping")[..., None]
        dx = x("off_d") * land_a * (1 + 0.5 * hy)
        dy = y("off_d") * land_b * (1 + 0.5 * hx)
        kx = x("off_k") * land_a
        ky = y("off_k") * land_b
        # Helping Hand: 1.5x damage (dx, dy include it), so a 0.65+ HP hit becomes a likely KO.
        kx = np.where(hy > 0, np.maximum(kx, np.clip((dx / hp_f - 0.95) / 0.2, 0, 1) * 0.9), kx)
        ky = np.where(hx > 0, np.maximum(ky, np.clip((dy / hp_f - 0.95) / 0.2, 0, 1) * 0.9), ky)
        dealt = np.minimum(dx + dy, hp_f)
        ko = 1 - (1 - kx) * (1 - ky)
        together = np.clip(((dx + dy) / hp_f - 0.8) / 0.4, 0, 1) * 0.9 * ((dx > 0.01) & (dy > 0.01))
        ko = np.maximum(ko, together)
        offense = (foe_on * (1 - pp) * (dealt + KO_VALUE * ko * (1 + c["foe_value"][i]) + FLINCH_VALUE * flinch)).sum(-1)
        ally = (x("ally_d") + LOSS_VALUE * x("ally_k")) * (1 - y("protect")) * land_a[..., 0]
        ally += (y("ally_d") + LOSS_VALUE * y("ally_k")) * (1 - x("protect")) * land_b[..., 0]
        defense = damage_a + damage_b + LOSS_VALUE * (loss_a + loss_b)
        bonus = x("bonus") + y("bonus") + x("act_bonus") * land_a[..., 0] + y("act_bonus") * land_b[..., 0]
        value = offense - defense - ALLY_WEIGHT * ally + bonus
        # Joint legality: one Tera per side, distinct switch targets.
        value = np.where(x("tera") & y("tera"), -np.inf, value)
        value = np.where((x("switch") >= 0) & (x("switch") == y("switch")), -np.inf, value)
        return value

    def _choose(self, values: np.ndarray) -> tuple[int, int]:
        values = values + NOISE * self.rng.random(values.shape)
        if self.temperature > 0:
            values = values + self.temperature * self.rng.gumbel(size=values.shape)
        if not np.isfinite(values).any():
            return 0, 0
        return np.unravel_index(int(np.argmax(values)), values.shape)

    def _replacement(self, c: dict, i: int, codes: np.ndarray, taken: int = -1) -> int:
        switches = codes[(codes >= 40) & (codes < 46) & (codes != taken)]
        if not len(switches):
            return int(codes[0]) if len(codes) else PASS
        score = c["matchup"][i, switches - 40] + NOISE * self.rng.random(len(switches))
        if not np.isfinite(score).any():  # Revival Blessing: every choice is a fainted Pokemon
            return int(self.rng.choice(switches))
        return int(switches[int(np.argmax(score))])

    # ---- agent interface ----------------------------------------------------------------------------------------
    def first(self, arrays, rows, mask0):
        obs = {k: np.asarray(arrays[k][rows]) for k in OBS_KEYS}
        c = self.ctx = self._prepare(obs)
        out = np.full(len(rows), PASS, dtype=np.int32)
        for i in range(len(rows)):
            codes = _codes_of(mask0[i])
            if not len(codes):
                continue
            if len(codes) == 1:
                out[i] = codes[0]
            elif not c["request_move"][i]:
                out[i] = self._replacement(c, i, codes)
            else:
                X = self._candidates(c, i, 0, codes)
                Y = self._candidates(c, i, 1, self._estimate_codes(c, i, 1))
                a, _ = self._choose(self._pair_values(c, i, X, Y))
                out[i] = X["code"][a]
        self.a0 = out
        return out

    def second(self, rows, mask1):
        c = self.ctx
        out = np.full(len(rows), PASS, dtype=np.int32)
        for i in range(len(rows)):
            codes = _codes_of(mask1[i])
            if not len(codes):
                continue
            if len(codes) == 1:
                out[i] = codes[0]
            elif not c["request_move"][i]:
                out[i] = self._replacement(c, i, codes, taken=int(self.a0[i]))
            else:
                X = self._candidates(c, i, 0, np.array([self.a0[i]]))
                Y = self._candidates(c, i, 1, codes)
                _, b = self._choose(self._pair_values(c, i, X, Y))
                out[i] = Y["code"][b]
        return out
