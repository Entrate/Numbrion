"""Static game data for the encoder and model: vocabularies, constant feature tables and set beliefs.

Everything comes from data/training/dex.json (tools/training/export-dex.mjs). Index 0 of every vocabulary means
"unknown or absent"; the item vocabulary also has NO_ITEM (the Pokemon holds nothing, for example after Knock Off).
"""
from __future__ import annotations

import json
import re
from functools import lru_cache
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[3]
DEX_PATH = REPO / "data" / "training" / "dex.json"

STATUSES = ("brn", "par", "slp", "frz", "psn", "tox")
STATS = ("hp", "atk", "def", "spa", "spd", "spe")
BOOSTS = ("atk", "def", "spa", "spd", "spe", "accuracy", "evasion")
TARGETS = ("normal", "any", "adjacentFoe", "adjacentAlly", "adjacentAllyOrSelf", "allAdjacent", "allAdjacentFoes",
           "self", "allySide", "foeSide", "all", "allies", "randomNormal", "scripted", "allyTeam")
MOVE_FLAGS = ("contact", "sound", "punch", "bite", "slicing", "pulse", "bullet", "wind", "protect", "bypasssub",
              "heal", "powder", "dance", "charge", "recharge")
SIDE_EFFECTS = ("tailwind", "reflect", "lightscreen", "auroraveil", "stealthrock", "spikes", "toxicspikes",
                "stickyweb", "wideguard", "quickguard", "safeguard", "mist")
VOLATILE_MOVES = ("followme", "ragepowder", "helpinghand", "encore", "taunt", "disable", "substitute", "yawn",
                  "leechseed", "imprison", "curse", "partiallytrapped", "noretreat")
NO_ITEM = 1

# Type multipliers granted by a defender's ability or item, applied to the attacking move's type.
ABILITY_TYPE_MULT = {
    "levitate": {"Ground": 0.0}, "eartheater": {"Ground": 0.0}, "flashfire": {"Fire": 0.0},
    "wellbakedbody": {"Fire": 0.0}, "waterabsorb": {"Water": 0.0}, "stormdrain": {"Water": 0.0},
    "dryskin": {"Water": 0.0, "Fire": 1.25}, "voltabsorb": {"Electric": 0.0}, "lightningrod": {"Electric": 0.0},
    "motordrive": {"Electric": 0.0}, "sapsipper": {"Grass": 0.0}, "thickfat": {"Fire": 0.5, "Ice": 0.5},
    "heatproof": {"Fire": 0.5}, "waterbubble": {"Fire": 0.5}, "purifyingsalt": {"Ghost": 0.5},
}
ITEM_TYPE_MULT = {"airballoon": {"Ground": 0.0}}


_NON_ID = re.compile(r"[^a-z0-9]")
_ID_CACHE: dict[str, str] = {}


def to_id(text: str) -> str:
    result = _ID_CACHE.get(text)
    if result is None:
        result = _ID_CACHE[text] = _NON_ID.sub("", text.lower())
    return result


class Dex:
    def __init__(self, path: Path = DEX_PATH):
        data = json.loads(Path(path).read_text(encoding="utf-8"))
        self.types = data["types"]  # 19 including Stellar
        self.type_index = {name: i + 1 for i, name in enumerate(self.types)}
        self.n_types = len(self.types) + 1
        chart = np.ones((self.n_types, self.n_types), dtype=np.float32)
        chart[1:, 1:] = np.asarray(data["typechart"], dtype=np.float32)
        self.typechart = chart  # [attacking, defending]

        self.species = data["species"]
        self.species_index = {s["id"]: i + 1 for i, s in enumerate(self.species)}
        self.n_species = len(self.species) + 1
        self.aliases = data["aliases"]
        self.species_by_id = {s["id"]: s for s in self.species}
        self.moves = data["moves"]
        self.move_index = {m["id"]: i + 1 for i, m in enumerate(self.moves)}
        self.n_moves = len(self.moves) + 1
        self.items = data["items"]
        self.item_index = {it["id"]: i + 2 for i, it in enumerate(self.items)}
        self.n_items = len(self.items) + 2
        self.abilities = data["abilities"]
        self.ability_index = {a["id"]: i + 1 for i, a in enumerate(self.abilities)}
        self.n_abilities = len(self.abilities) + 1
        self.sets = data["sets"]
        self._build_species_tables()
        self._build_move_tables()
        self._build_multipliers()

    # ---- name resolution -------------------------------------------------------------------------------------
    def species_id(self, name: str) -> str:
        sid = to_id(name)
        return self.aliases.get(sid, sid)

    def species_idx(self, name: str) -> int:
        return self.species_index.get(self.species_id(name), 0)

    def move_idx(self, name: str) -> int:
        return self.move_index.get(to_id(name), 0)

    def item_idx(self, name: str) -> int:
        return self.item_index.get(to_id(name), 0) if name else NO_ITEM

    def ability_idx(self, name: str) -> int:
        return self.ability_index.get(to_id(name), 0)

    def type_idx(self, name: str) -> int:
        return self.type_index.get(name, 0)

    # ---- constant tables -------------------------------------------------------------------------------------
    def _build_species_tables(self):
        n = self.n_species
        self.species_base = np.zeros((n, 6), dtype=np.float32)
        self.species_types = np.zeros((n, 2), dtype=np.int64)
        self.species_weight = np.zeros(n, dtype=np.float32)
        for i, s in enumerate(self.species, start=1):
            self.species_base[i] = [s["baseStats"][k] for k in STATS]
            for j, t in enumerate(s["types"][:2]):
                self.species_types[i, j] = self.type_idx(t)
            self.species_weight[i] = s["weightkg"]

    def _build_move_tables(self):
        n = self.n_moves
        self.move_type = np.zeros(n, dtype=np.int64)
        self.move_category = np.zeros(n, dtype=np.int64)  # 0 status, 1 physical, 2 special
        self.move_power = np.zeros(n, dtype=np.float32)
        self.move_accuracy = np.ones(n, dtype=np.float32)
        self.move_priority = np.zeros(n, dtype=np.float32)
        self.move_target = np.zeros(n, dtype=np.int64)
        self.move_hits = np.ones(n, dtype=np.float32)
        self.move_special_power = np.zeros(n, dtype=np.int64)  # see damage.py SPECIAL_POWER
        self.move_stalling = np.zeros(n, dtype=bool)
        rows = []
        for i, m in enumerate(self.moves, start=1):
            self.move_type[i] = self.type_idx(m["type"])
            self.move_category[i] = ("Status", "Physical", "Special").index(m["category"])
            power = float(m["basePower"])
            if m["category"] != "Status" and power <= 1:
                power = 60.0  # variable power without a closed form here; damage.py overrides the common ones
            self.move_power[i] = power
            self.move_accuracy[i] = min(m["accuracy"], 100) / 100
            self.move_priority[i] = m["priority"]
            self.move_target[i] = TARGETS.index(m["target"]) if m["target"] in TARGETS else 0
            hits = m["multihit"]
            if isinstance(hits, list):
                self.move_hits[i] = 3.1 if hits == [2, 5] else (hits[0] + hits[1]) / 2
            elif hits:
                self.move_hits[i] = float(hits)
            if m["id"] == "tripleaxel":
                self.move_hits[i] = 5.4  # 20+40+60 power is six 20-power hits; ~90% of the follow-ups land
            self.move_special_power[i] = SPECIAL_POWER.get(m["id"], 0)
            self.move_stalling[i] = m["stallingMove"]
            rows.append(self._move_features(m))
        width = len(rows[0])
        self.move_features = np.zeros((n, width), dtype=np.float32)
        self.move_features[1:] = np.asarray(rows, dtype=np.float32)

    def _move_features(self, m: dict) -> list[float]:
        f: list[float] = []
        f += [float(m["type"] == t) for t in self.types]
        f += [float(m["category"] == c) for c in ("Physical", "Special", "Status")]
        f += [m["basePower"] / 150, float(m["variablePower"] or m["damage"] is not None)]
        f += [min(m["accuracy"], 100) / 100, float(m["accuracy"] > 100)]
        f += [m["priority"] / 5, float(m["priority"] > 0), float(m["priority"] < 0)]
        f += [float(m["target"] == t) for t in TARGETS]
        f += [float(flag in m["flags"]) for flag in MOVE_FLAGS]
        hits = m["multihit"]
        f += [(3.1 if hits == [2, 5] else (sum(hits) / 2 if isinstance(hits, list) else (hits or 1))) / 5]
        f += [m["drain"], m["recoil"], m["heal"], float(m["willCrit"] or m["critRatio"] > 1)]
        f += [float(m[k]) for k in ("selfSwitch", "forceSwitch", "breaksProtect", "stallingMove", "selfdestruct")]
        f += [m["secondaryChance"], m["pp"] / 40]
        status = m["status"] or m["secondaryStatus"]
        chance = 1.0 if m["status"] else m["secondaryChance"]
        f += [chance * float(status == s) for s in STATUSES]
        volatile = m["volatileStatus"] or m["secondaryVolatile"]
        f += [float(volatile == v) for v in VOLATILE_MOVES] + [float(volatile == "flinch") * m["secondaryChance"]]
        f += [float(m["sideCondition"] == s) for s in SIDE_EFFECTS]
        f += [float(m["weather"] is not None), float(m["terrain"] is not None), float(m["pseudoWeather"] == "trickroom")]
        target_boosts = m["boosts"] or {}
        secondary_boosts = m["secondaryBoosts"] or {}
        self_boosts = m["selfBoosts"] or {}
        f += [target_boosts.get(k, 0) / 2 + secondary_boosts.get(k, 0) * m["secondaryChance"] / 2 for k in BOOSTS[:5]]
        f += [self_boosts.get(k, 0) / 2 for k in BOOSTS[:5]]
        return f

    def _build_multipliers(self):
        self.ability_type_mult = np.ones((self.n_abilities, self.n_types), dtype=np.float32)
        for ability, table in ABILITY_TYPE_MULT.items():
            if ability in self.ability_index:
                for t, mult in table.items():
                    self.ability_type_mult[self.ability_index[ability], self.type_idx(t)] = mult
        self.item_type_mult = np.ones((self.n_items, self.n_types), dtype=np.float32)
        for item, table in ITEM_TYPE_MULT.items():
            if item in self.item_index:
                for t, mult in table.items():
                    self.item_type_mult[self.item_index[item], self.type_idx(t)] = mult

    # ---- random-battle priors --------------------------------------------------------------------------------
    def base_species(self, sid: str) -> str:
        entry = self.species_by_id.get(sid)
        while entry and entry["battleOnly"] and entry["battleOnly"] in self.species_by_id:
            sid = entry["battleOnly"]
            entry = self.species_by_id[sid]
        return sid

    @lru_cache(maxsize=None)
    def prior(self, sid: str) -> dict:
        """Team-generator marginals and role sets for a species id (battle-only formes use their base)."""
        base = self.base_species(sid)
        entry = self.species_by_id.get(base) or {}
        sets = []
        for s in self.sets.get(base, []):
            sets.append(dict(
                weight=s["weight"],
                moves=frozenset(self.move_index[m] for m in s["movepool"] if m in self.move_index),
                abilities=frozenset(self.ability_index[a] for a in s["abilities"] if a in self.ability_index),
                tera=frozenset(self.type_idx(t) for t in s["teraTypes"]),
            ))
        return dict(
            level=entry.get("level") or 80,
            moves={self.move_index[k]: v for k, v in (entry.get("moves") or {}).items() if k in self.move_index},
            items={self.item_index[k]: v for k, v in (entry.get("items") or {}).items() if k in self.item_index},
            abilities={self.ability_index[k]: v for k, v in (entry.get("abilities") or {}).items() if k in self.ability_index},
            tera={self.type_idx(k): v for k, v in (entry.get("teraTypes") or {}).items()},
            sets=sets,
        )

    @lru_cache(maxsize=200_000)
    def belief(self, sid: str, known_moves: frozenset, ability: int, tera: int) -> tuple:
        """Posterior over unrevealed moves (inclusion probabilities), abilities and Tera types.

        Sets consistent with every revealed move, ability and Tera type are weighted by their role share times the
        chance that a uniformly drawn 4-move subset of the movepool contains the revealed moves; inside a set every
        unrevealed movepool move then has probability (4 - r) / (n - r). Falls back to team-generator marginals.
        """
        prior = self.prior(sid)
        consistent = []
        for s in prior["sets"]:
            if not known_moves <= s["moves"]:
                continue
            if (ability and s["abilities"] and ability not in s["abilities"]) or (tera and s["tera"] and tera not in s["tera"]):
                continue
            n, r = len(s["moves"]), len(known_moves)
            picks = min(4, n)
            likelihood = 1.0
            for k in range(r):
                likelihood *= max(picks - k, 0) / max(n - k, 1)
            if likelihood > 0:
                consistent.append((s["weight"] * likelihood, s, (picks - r) / max(n - r, 1)))
        moves: dict[int, float] = {}
        tera_probs: dict[int, float] = {}
        total = sum(w for w, _, _ in consistent)
        for w, s, p in consistent:
            share = w / total
            for m in s["moves"] - known_moves:
                moves[m] = moves.get(m, 0.0) + share * p
            for t in s["tera"]:
                tera_probs[t] = tera_probs.get(t, 0.0) + share / len(s["tera"])
        if not consistent:
            missing = max(4 - len(known_moves), 0)
            free = sum(p for m, p in prior["moves"].items() if m not in known_moves) or 1.0
            moves = {m: min(1.0, p * missing / free) for m, p in prior["moves"].items() if m not in known_moves}
        abilities = prior["abilities"]  # team-generator marginals; the generator does not pick set abilities uniformly
        if not tera_probs:
            tera_probs = prior["tera"]
        top_moves = sorted(moves.items(), key=lambda kv: -kv[1])
        return tuple(top_moves), tuple(abilities.items()), tuple(tera_probs.items()), tuple(prior["items"].items())


# Variable-power moves with a closed form the damage features compute (damage.py).
SPECIAL_POWER = {"lowkick": 1, "grassknot": 1, "heavyslam": 2, "heatcrash": 2, "eruption": 3, "waterspout": 3,
                 "dragonenergy": 3, "nightshade": 4, "superfang": 5, "ruination": 5, "endeavor": 6, "finalgambit": 6}


@lru_cache(maxsize=1)
def load_dex() -> Dex:
    return Dex()
