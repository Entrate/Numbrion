"""Token observation encoder built only from a side's own request and its player-view protocol (TIPS rule 1).

Each row (one side of one battle) becomes 12 Pokemon tokens and one field token:

* tokens 0-5: own team in request order (0 and 1 are the active slots a and b),
* token 6 / 7: the foe in active slot a / b, tokens 8-11: other revealed foes in reveal order,
* the field token (weather, terrain, rooms, side conditions, turn, request kind).

Integer ids feed learned embeddings; floats carry HP, status, stats, boosts, timing and what has been revealed;
belief bags give each foe's likely unrevealed moves, abilities, items and Tera types (training.dex.Dex.belief).
The tracker never sees the omniscient log or the opponent's request. ``encode_batch`` additionally returns the
opponent-row pairing used by the training-only oracle critic and auxiliary targets.
"""
from __future__ import annotations

import json
from dataclasses import dataclass, field

import numpy as np

from .dex import BOOSTS, NO_ITEM, STATUSES, Dex, load_dex, to_id

N_TOKENS = 12
OWN, FOE_A, FOE_B = 0, 6, 7
K_MOVES, K_ABILITIES, K_ITEMS = 10, 4, 4
VOLATILES = ("substitute", "confusion", "taunt", "encore", "disable", "yawn", "leechseed", "saltcure",
             "focusenergy", "partiallytrapped")
PERISH = {"perish3": 3, "perish2": 2, "perish1": 1, "perish0": 0}
WEATHERS = {"sunnyday": 0, "desolateland": 0, "raindance": 1, "primordialsea": 1, "sandstorm": 2, "snowscape": 3,
            "snow": 3, "hail": 3, "deltastream": 4}
EXTREME_WEATHER = ("desolateland", "primordialsea", "deltastream")
TERRAINS = ("electricterrain", "grassyterrain", "mistyterrain", "psychicterrain")
SIDE_CONDITIONS = ("tailwind", "reflect", "lightscreen", "auroraveil", "stealthrock", "spikes", "toxicspikes",
                   "stickyweb", "safeguard", "mist")
TIMED_SIDE = ("tailwind", "reflect", "lightscreen", "auroraveil")

# Per-token floats: a dynamic block rewritten every step, then a static block cached per Pokemon state.
DYNAMIC_FLOATS = (
    ["present", "own", "active_a", "active_b", "bench", "hp", "fainted"] + list(STATUSES)
    + [f"boost_{b}" for b in BOOSTS]
    + ["terastallized", "tera_available", "switch_turns", "fresh", "protect_last", "trapped", "perish", "stat_hp"]
    + list(VOLATILES) + [f"pp{m}" for m in range(4)] + [f"disabled{m}" for m in range(4)]
)
STATIC_FLOATS = (
    ["level", "stat_atk", "stat_def", "stat_spa", "stat_spd", "stat_spe", "item_known", "ability_known", "tera_known",
     "moves_seen", "item_lost"] + [f"move_known{m}" for m in range(4)]
    + [f"cand_move_p{j}" for j in range(K_MOVES)] + [f"cand_ability_p{j}" for j in range(K_ABILITIES)]
    + [f"cand_item_p{j}" for j in range(K_ITEMS)] + [f"tera_p{t}" for t in range(20)] + [f"type_mult{t}" for t in range(20)]
)
POKEMON_FLOATS = DYNAMIC_FLOATS + STATIC_FLOATS
PF = {name: i for i, name in enumerate(POKEMON_FLOATS)}
N_DYNAMIC = len(DYNAMIC_FLOATS)
N_POKEMON_FLOATS = len(POKEMON_FLOATS)
FIELD_FLOATS = (
    ["sun", "rain", "sand", "snow", "extreme_weather", "weather_turns"] + list(TERRAINS) + ["terrain_turns"]
    + ["trickroom", "trickroom_turns", "gravity"]
    + [f"own_{c}" for c in SIDE_CONDITIONS] + [f"own_{c}_turns" for c in TIMED_SIDE]
    + [f"foe_{c}" for c in SIDE_CONDITIONS] + [f"foe_{c}_turns" for c in TIMED_SIDE]
    + ["turn", "own_tera_used", "foe_tera_used", "request_move", "request_switch", "request_wait",
       "own_alive", "foe_fainted", "foe_revealed", "foe_unrevealed"]
)
FF = {name: i for i, name in enumerate(FIELD_FLOATS)}
N_FIELD_FLOATS = len(FIELD_FLOATS)
# Integer ids per token: identity, four move slots, then the belief candidates.
I_SPECIES, I_ITEM, I_ABILITY, I_TERA, I_TYPE1, I_TYPE2, I_MOVE0 = 0, 1, 2, 3, 4, 5, 6
I_CAND_MOVES = 10
I_CAND_ABILITIES = I_CAND_MOVES + K_MOVES
I_CAND_ITEMS = I_CAND_ABILITIES + K_ABILITIES
N_IDS = I_CAND_ITEMS + K_ITEMS
STAT_SCALE = 400.0
CACHE_LIMIT = 50_000  # entries per static-feature cache (~60 MB); foe reveal states keep growing otherwise
N_ACTION_FEATURES = 9
N_TOKEN_FEATURES = 6


def span(name: str, length: int) -> slice:
    return slice(PF[name], PF[name] + length)


def condition(text: str) -> tuple[float, float, str]:
    """``"169/260 par"`` -> (hp, max hp, status); fainted gives (0, max, "fnt")."""
    parts = text.split()
    if not parts:
        return 0.0, 1.0, "fnt"
    if parts[0] == "0" or "fnt" in parts:
        return 0.0, 1.0, "fnt"
    hp = parts[0].split("/")
    status = parts[1] if len(parts) > 1 else ""
    if len(hp) != 2:
        return 1.0, 1.0, status
    return float(hp[0]), max(float(hp[1]), 1.0), status


def details(text: str) -> tuple[str, int, str]:
    """``"Inteleon, L78, F, tera:Water"`` -> (species, level, tera type or "")."""
    parts = [p.strip() for p in text.split(",")]
    level, tera = 100, ""
    for p in parts[1:]:
        if p.startswith("L") and p[1:].isdigit():
            level = int(p[1:])
        elif p.startswith("tera:"):
            tera = p[5:]
    return parts[0], level, tera


@dataclass
class Mon:
    species: str = ""
    level: int = 100
    hp: float = 1.0
    status: str = ""
    boosts: dict = field(default_factory=dict)
    volatiles: dict = field(default_factory=dict)
    switch_turn: int = 0
    moves_since_switch: int = 0
    protect_turn: int = -10
    moves: list = field(default_factory=list)  # revealed move ids in order
    item: str | None = None  # None unknown, "" none/lost, else item id
    item_lost: bool = False
    ability: str | None = None
    tera: str = ""
    terastallized: bool = False
    # Illusion bookkeeping: how often the name has switched in, what was known about it before the current switch-in,
    # every move used during the stint, and the moves the stint added to ``moves`` (a broken disguise hands the
    # stint's observations to the real Pokemon and gives the name back what it had).
    appearances: int = 0
    before_stint: dict | None = None
    stint_moves: list = field(default_factory=list)
    stint_added: list = field(default_factory=list)


KNOWLEDGE = ("hp", "status", "item", "item_lost", "ability", "tera", "terastallized")


class Tracker:
    """Public battle state of one row, updated from that side's player-view protocol lines."""

    stalling: frozenset = frozenset({"protect", "detect", "spikyshield", "banefulbunker", "kingsshield", "silktrap",
                                     "obstruct", "burningbulwark"})

    def __init__(self, side: int):
        self.side = side
        self.own = f"p{side + 1}"
        self.reset(-1)

    def reset(self, battle_id: int) -> None:
        self.battle_id = battle_id
        self.turn = 0
        self.mons: dict[tuple[str, str], Mon] = {}
        self.active: dict[str, tuple[str, str]] = {}
        self.foe_order: list[tuple[str, str]] = []
        self.weather = ("", 0)
        self.terrain = ("", 0)
        self.trickroom = -1
        self.gravity = False
        self.sides = {"p1": {}, "p2": {}}  # condition -> (start turn, layers)
        self.tera_used = {"p1": False, "p2": False}

    # ---- protocol ------------------------------------------------------------------------------------------
    def mon(self, ident: str) -> Mon | None:
        side, _, name = ident.partition(": ")
        if not name or len(side) < 2:
            return None
        key = (side[:2], name)
        mon = self.mons.get(key)
        if mon is None and side[:2] in ("p1", "p2"):
            mon = self.mons[key] = Mon()
            if side[:2] != self.own:
                self.foe_order.append(key)
        return mon

    def feed(self, lines: list[str]) -> None:
        for line in lines:
            if len(line) < 2 or line[0] != "|":
                continue
            parts = line.split("|")
            event = parts[1]
            handler = HANDLERS.get(event)
            if handler is not None:
                try:
                    handler(self, parts)
                except (IndexError, ValueError):
                    pass
            if "[from] ability:" in line or "[from] item:" in line:
                self._attribute(parts)

    def _attribute(self, parts: list[str]) -> None:
        source, owner = None, None
        for p in parts[2:]:
            if p.startswith("[from] ability: ") or p.startswith("[from] item: "):
                source = p
            elif p.startswith("[of] "):
                owner = p[5:]
        if source is None:
            return
        if owner is None and len(parts) > 2 and parts[2][:2] in ("p1", "p2") and ": " in parts[2]:
            owner = parts[2]
        mon = self.mon(owner) if owner else None
        if mon is None:
            return
        kind, _, name = source[7:].partition(": ")
        if kind == "ability":
            mon.ability = to_id(name)
        elif kind == "item" and mon.item is None:
            mon.item = to_id(name)

    def _switch(self, parts):
        position = parts[2].split(":", 1)[0]
        if parts[1] == "replace":
            self._reveal_illusion(position[:3], parts[2])
        key_mon = self.mon(parts[2])
        if key_mon is None:
            return
        if parts[1] != "replace":
            key_mon.before_stint = {k: getattr(key_mon, k) for k in KNOWLEDGE}
            key_mon.appearances += 1
            key_mon.stint_moves, key_mon.stint_added = [], []
        species, level, tera = details(parts[3])
        key_mon.species, key_mon.level = species, level
        if tera:
            key_mon.tera, key_mon.terastallized = tera, True
        if len(parts) > 4:
            hp, max_hp, status = condition(parts[4])
            key_mon.hp, key_mon.status = hp / max_hp, status
        if parts[1] != "replace":
            key_mon.boosts, key_mon.volatiles = {}, {}
            key_mon.switch_turn, key_mon.moves_since_switch = self.turn, 0
        self.active[position[:3]] = (position[:2], parts[2].split(": ", 1)[1])

    def _reveal_illusion(self, position: str, ident: str) -> None:
        """``|replace|`` (Illusion broke): what was observed under the disguise belongs to the real Pokemon.

        The real Pokemon gets the stint's HP, status, boosts, volatiles, timing, every move used, and any item, ability
        or Tera observed during the stint (the ``replace`` line itself carries no HP); its ability is Illusion. The
        disguise's record keeps none of it: it is dropped when that switch-in was its only appearance, otherwise what
        was known before the switch-in returns.

        Known limits (Illusion is rare: the two Zoroark formes): an earlier stint that was also disguised and ended
        unrevealed stays attributed to the disguise; the real and disguised Pokemon active at the same time share one
        record; a disguised Pokemon fainting unrevealed is counted on the disguise; and our own disguised Pokemon's
        boosts are only matched to its request entry after the reveal.
        """
        old_key = self.active.get(position)
        real = self.mon(ident)
        new_key = (ident[:2], ident.split(": ", 1)[1])
        if old_key is None or old_key == new_key or old_key not in self.mons or real is None:
            return
        disguise = self.mons[old_key]
        before = disguise.before_stint or {k: getattr(Mon(), k) for k in KNOWLEDGE}
        real.hp, real.status = disguise.hp, disguise.status
        for key in KNOWLEDGE[2:]:  # item, ability and Tera knowledge gained during the stint
            if getattr(disguise, key) != before[key]:
                setattr(real, key, getattr(disguise, key))
        real.ability = "illusion"
        real.boosts, real.volatiles = disguise.boosts, disguise.volatiles
        real.switch_turn, real.moves_since_switch = disguise.switch_turn, disguise.moves_since_switch
        real.protect_turn = disguise.protect_turn
        real.appearances += 1
        real.before_stint = None
        real.stint_moves, real.stint_added = list(disguise.stint_moves), []
        for move in disguise.stint_moves:
            if move not in real.moves and len(real.moves) < 4:
                real.moves.append(move)
                real.stint_added.append(move)
        if disguise.appearances <= 1:
            del self.mons[old_key]
            if old_key in self.foe_order:
                self.foe_order.remove(old_key)
        else:
            disguise.appearances -= 1
            for key in KNOWLEDGE:
                setattr(disguise, key, before[key])
            disguise.moves = [m for m in disguise.moves if m not in disguise.stint_added]
            disguise.boosts, disguise.volatiles, disguise.stint_moves, disguise.stint_added = {}, {}, [], []

    def _detailschange(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            species, level, tera = details(parts[3])
            mon.species = species
            if tera:
                mon.tera, mon.terastallized = tera, True

    def _hp(self, parts):
        mon = self.mon(parts[2])
        if mon is not None and len(parts) > 3:
            hp, max_hp, status = condition(parts[3])
            mon.hp, mon.status = hp / max_hp, status

    def _faint(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            mon.hp, mon.status = 0.0, "fnt"

    def _status(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            mon.status = parts[3]

    def _curestatus(self, parts):
        mon = self.mon(parts[2])
        if mon is not None and mon.status != "fnt":
            mon.status = ""

    def _cureteam(self, parts):
        side = parts[2][:2]
        for (s, _), mon in self.mons.items():
            if s == side and mon.status != "fnt":
                mon.status = ""

    def _boost(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            sign = 1 if parts[1] == "-boost" else -1
            stat = parts[3]
            mon.boosts[stat] = max(-6, min(6, mon.boosts.get(stat, 0) + sign * int(parts[4])))

    def _setboost(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            mon.boosts[parts[3]] = max(-6, min(6, int(parts[4])))

    def _clearboost(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            if parts[1] == "-clearnegativeboost":
                mon.boosts = {k: v for k, v in mon.boosts.items() if v > 0}
            elif parts[1] == "-clearpositiveboost":
                mon.boosts = {k: v for k, v in mon.boosts.items() if v < 0}
            elif parts[1] == "-invertboost":
                mon.boosts = {k: -v for k, v in mon.boosts.items()}
            else:
                mon.boosts = {}

    def _clearallboost(self, parts):
        for mon in self.mons.values():
            mon.boosts = {}

    def _copyboost(self, parts):
        source, target = self.mon(parts[2]), self.mon(parts[3])
        if source is not None and target is not None:
            source.boosts = dict(target.boosts)

    def _move(self, parts):
        mon = self.mon(parts[2])
        if mon is None:
            return
        move = to_id(parts[3])
        mon.moves_since_switch += 1
        if move in self.stalling:
            mon.protect_turn = self.turn
        if move == "struggle" or "[from]" in "".join(parts[4:]):
            return
        if move not in mon.stint_moves:
            mon.stint_moves.append(move)
        if move not in mon.moves and len(mon.moves) < 4:
            mon.moves.append(move)
            mon.stint_added.append(move)

    def _item(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            mon.item, mon.item_lost = to_id(parts[3]), False

    def _enditem(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            mon.item, mon.item_lost = "", True

    def _ability(self, parts):
        mon = self.mon(parts[2])
        if mon is not None and len(parts) > 3:
            mon.ability = to_id(parts[3])

    def _terastallize(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            mon.tera, mon.terastallized = parts[3], True
            self.tera_used[parts[2][:2]] = True

    def _start(self, parts):
        mon = self.mon(parts[2])
        if mon is None:
            return
        effect = to_id(parts[3].removeprefix("move: ").removeprefix("ability: "))
        if effect in PERISH:
            mon.volatiles["perish"] = PERISH[effect]
        elif effect in VOLATILE_SET:
            mon.volatiles[effect] = self.turn

    def _end(self, parts):
        mon = self.mon(parts[2])
        if mon is not None:
            effect = to_id(parts[3].removeprefix("move: ").removeprefix("ability: "))
            mon.volatiles.pop(effect, None)

    def _weather(self, parts):
        name = to_id(parts[2])
        if name == "none":
            self.weather = ("", 0)
        elif "[upkeep]" not in parts and name != self.weather[0]:
            self.weather = (name, self.turn)

    def _fieldstart(self, parts):
        name = to_id(parts[2].removeprefix("move: "))
        if name in TERRAINS:
            self.terrain = (name, self.turn)
        elif name == "trickroom":
            self.trickroom = self.turn
        elif name == "gravity":
            self.gravity = True

    def _fieldend(self, parts):
        name = to_id(parts[2].removeprefix("move: "))
        if name in TERRAINS:
            self.terrain = ("", 0)
        elif name == "trickroom":
            self.trickroom = -1
        elif name == "gravity":
            self.gravity = False

    def _sidestart(self, parts):
        side = parts[2][:2]
        name = to_id(parts[3].removeprefix("move: "))
        start, layers = self.sides[side].get(name, (self.turn, 0))
        self.sides[side][name] = (start, layers + 1)

    def _sideend(self, parts):
        self.sides[parts[2][:2]].pop(to_id(parts[3].removeprefix("move: ")), None)

    def _turn(self, parts):
        self.turn = int(parts[2])

    def _swap(self, parts):
        side = parts[2][:2]
        a, b = self.active.get(side + "a"), self.active.get(side + "b")
        self.active[side + "a"], self.active[side + "b"] = b, a


HANDLERS = {
    "switch": Tracker._switch, "drag": Tracker._switch, "replace": Tracker._switch,
    "detailschange": Tracker._detailschange, "-formechange": Tracker._detailschange,
    "-damage": Tracker._hp, "-heal": Tracker._hp, "-sethp": Tracker._hp, "faint": Tracker._faint,
    "-status": Tracker._status, "-curestatus": Tracker._curestatus, "-cureteam": Tracker._cureteam,
    "-boost": Tracker._boost, "-unboost": Tracker._boost, "-setboost": Tracker._setboost,
    "-clearboost": Tracker._clearboost, "-clearnegativeboost": Tracker._clearboost,
    "-clearpositiveboost": Tracker._clearboost, "-invertboost": Tracker._clearboost,
    "-clearallboost": Tracker._clearallboost, "-copyboost": Tracker._copyboost,
    "move": Tracker._move, "-item": Tracker._item, "-enditem": Tracker._enditem, "-ability": Tracker._ability,
    "-terastallize": Tracker._terastallize, "-start": Tracker._start, "-end": Tracker._end,
    "-weather": Tracker._weather, "-fieldstart": Tracker._fieldstart, "-fieldend": Tracker._fieldend,
    "-sidestart": Tracker._sidestart, "-sideend": Tracker._sideend, "turn": Tracker._turn, "swap": Tracker._swap,
}
VOLATILE_SET = frozenset(VOLATILES)


def estimated_stats(base: np.ndarray, level: int) -> np.ndarray:
    """Random-battle stats: 31 IVs, 85 EVs, neutral nature."""
    core = np.floor((2 * base + 31 + 21) * level / 100)
    stats = core + 5
    stats[0] = core[0] + level + 10
    return stats


class Buffers:
    """Preallocated observation arrays for ``rows`` rows."""

    KEYS = ("ids", "floats", "field", "foe_match", "action_features", "token_features")

    def __init__(self, rows: int):
        self.rows = rows
        self.ids = np.zeros((rows, N_TOKENS, N_IDS), dtype=np.int64)
        self.floats = np.zeros((rows, N_TOKENS, N_POKEMON_FLOATS), dtype=np.float32)
        self.field = np.zeros((rows, N_FIELD_FLOATS), dtype=np.float32)
        # Training-only: for foe tokens 6-11, the index of the same Pokemon among the opponent row's own tokens.
        self.foe_match = np.full((rows, 6), -1, dtype=np.int64)
        self.fainted = np.zeros((rows, 2), dtype=np.float32)  # own, foe (reward shaping)
        # Damage-calc features (training.damage), filled by Encoder.encode_batch.
        self.action_features = np.zeros((rows, 2, 40, N_ACTION_FEATURES), dtype=np.float32)
        self.token_features = np.zeros((rows, N_TOKENS, N_TOKEN_FEATURES), dtype=np.float32)

    def arrays(self) -> dict:
        return {k: getattr(self, k) for k in self.KEYS}

    def copy(self) -> dict:
        return {k: getattr(self, k).copy() for k in self.KEYS}


_P = PF
(D_PRESENT, D_OWN, D_ACTIVE_A, D_ACTIVE_B, D_BENCH, D_HP, D_FAINTED, D_TERASTALLIZED, D_TERA_AVAILABLE,
 D_SWITCH_TURNS, D_FRESH, D_PROTECT_LAST, D_TRAPPED, D_PERISH, D_STAT_HP, D_PP0, D_DISABLED0) = (
    PF[k] for k in ("present", "own", "active_a", "active_b", "bench", "hp", "fainted", "terastallized",
                    "tera_available", "switch_turns", "fresh", "protect_last", "trapped", "perish", "stat_hp",
                    "pp0", "disabled0"))
_ZERO_DYNAMIC = [0.0] * N_DYNAMIC
_ZERO_STATIC = np.zeros(N_POKEMON_FLOATS - N_DYNAMIC, dtype=np.float32)
_ZERO_IDS = np.zeros(N_IDS, dtype=np.int64)
_STATUS_INDEX = {s: PF[s] for s in STATUSES}
_BOOST_INDEX = {b: PF["boost_" + b] for b in BOOSTS}
_VOLATILE_INDEX = {v: PF[v] for v in VOLATILES}


class Encoder:
    """Encodes rows ``2 * env + side`` of a ``BatchEnv`` (or any source of observe() dicts)."""

    def __init__(self, rows: int, dex: Dex | None = None):
        self.dex = dex or load_dex()
        self.trackers = [Tracker(r % 2) for r in range(rows)]
        self.buffers = Buffers(rows)
        self.own_names: list[list[str]] = [[] for _ in range(rows)]
        self.foe_names: list[list] = [[None] * 6 for _ in range(rows)]
        self._own_cache: dict = {}
        self._foe_cache: dict = {}
        from .damage import DamageFeatures  # torch, CPU only
        self.damage = DamageFeatures(self.dex)

    def encode_batch(self, observations: list[dict]) -> Buffers:
        for row, observation in enumerate(observations):
            self.encode(row, observation)
        self.match_partners()
        self.damage_features()
        return self.buffers

    def damage_features(self) -> None:
        import torch
        b = self.buffers
        with torch.no_grad():
            action, token = self.damage(torch.from_numpy(b.ids), torch.from_numpy(b.floats), torch.from_numpy(b.field))
        b.action_features[:] = action.numpy()
        b.token_features[:] = token.numpy()

    def match_partners(self) -> None:
        b = self.buffers
        b.foe_match.fill(-1)
        for row in range(0, b.rows - 1, 2):
            for me, other in ((row, row + 1), (row + 1, row)):
                lookup = {name: j for j, name in enumerate(self.own_names[other])}
                for k, name in enumerate(self.foe_names[me]):
                    if name is not None:
                        b.foe_match[me, k] = lookup.get(name, -1)

    # ---- static parts, cached per Pokemon state --------------------------------------------------------------
    def _own_static(self, pokemon: dict):
        key = (pokemon["details"], pokemon.get("item", ""), pokemon.get("ability", ""), pokemon.get("teraType", ""),
               tuple(pokemon.get("moves", ())), tuple(pokemon.get("stats", {}).values()))
        cached = self._own_cache.get(key)
        if cached is not None:
            return cached
        dex = self.dex
        species, level, _ = details(pokemon["details"])
        sp = dex.species_idx(species)
        ids = np.zeros(N_IDS, dtype=np.int64)
        ids[I_SPECIES] = sp
        ids[I_ITEM] = dex.item_idx(pokemon.get("item", ""))
        ids[I_ABILITY] = dex.ability_idx(pokemon.get("ability") or pokemon.get("baseAbility", ""))
        ids[I_TERA] = dex.type_idx(pokemon.get("teraType", ""))
        ids[I_TYPE1:I_TYPE2 + 1] = dex.species_types[sp]
        moves = pokemon.get("moves", [])[:4]
        for m, move in enumerate(moves):
            ids[I_MOVE0 + m] = dex.move_index.get(move, 0)
        row = np.zeros(N_POKEMON_FLOATS, dtype=np.float32)
        row[_P["level"]] = level / 100
        stats = pokemon.get("stats", {})
        for s in ("atk", "def", "spa", "spd", "spe"):
            row[_P["stat_" + s]] = stats.get(s, 0) / STAT_SCALE
        row[_P["item_known"]] = row[_P["ability_known"]] = row[_P["tera_known"]] = row[_P["moves_seen"]] = 1
        row[_P["item_lost"]] = float(not pokemon.get("item"))
        row[_P["move_known0"]:_P["move_known0"] + len(moves)] = 1
        row[_P["tera_p0"] + ids[I_TERA]] = 1
        row[span("type_mult0", 20)] = dex.ability_type_mult[ids[I_ABILITY]] * dex.item_type_mult[ids[I_ITEM]]
        max_hp = float(estimated_stats(dex.species_base[sp], level)[0])
        if len(self._own_cache) >= CACHE_LIMIT:
            self._own_cache.clear()
        cached = self._own_cache[key] = (ids, row[N_DYNAMIC:].copy(), max_hp)
        return cached

    def _foe_static(self, mon: Mon):
        key = (mon.species, mon.level, tuple(mon.moves), mon.ability, mon.tera if mon.terastallized else "",
               mon.item, mon.item_lost)
        cached = self._foe_cache.get(key)
        if cached is not None:
            return cached
        dex = self.dex
        sid = dex.species_id(mon.species)
        sp = dex.species_index.get(sid, 0)
        ids = np.zeros(N_IDS, dtype=np.int64)
        ids[I_SPECIES] = sp
        ids[I_TYPE1:I_TYPE2 + 1] = dex.species_types[sp]
        move_ids = [dex.move_index.get(m, 0) for m in mon.moves[:4]]
        ids[I_MOVE0:I_MOVE0 + len(move_ids)] = move_ids
        ability = dex.ability_index.get(mon.ability, 0) if mon.ability else 0
        tera = dex.type_idx(mon.tera) if mon.terastallized else 0
        ids[I_ABILITY], ids[I_TERA] = ability, tera
        if mon.item is not None:
            ids[I_ITEM] = dex.item_index.get(mon.item, 0) if mon.item else NO_ITEM
        row = np.zeros(N_POKEMON_FLOATS, dtype=np.float32)
        stats = estimated_stats(dex.species_base[sp], mon.level)
        row[_P["level"]] = mon.level / 100
        row[span("stat_atk", 5)] = stats[1:] / STAT_SCALE
        row[_P["item_known"]] = float(mon.item is not None)
        row[_P["ability_known"]] = float(bool(ability))
        row[_P["tera_known"]] = float(bool(tera))
        row[_P["moves_seen"]] = len(move_ids) / 4
        row[_P["item_lost"]] = float(mon.item_lost)
        row[_P["move_known0"]:_P["move_known0"] + len(move_ids)] = 1
        moves, abilities, teras, items = dex.belief(sid, frozenset(m for m in move_ids if m), ability, tera)
        for j, (m, p) in enumerate(moves[:K_MOVES]):
            ids[I_CAND_MOVES + j], row[_P["cand_move_p0"] + j] = m, p
        if not ability:
            for j, (a, p) in enumerate(abilities[:K_ABILITIES]):
                ids[I_CAND_ABILITIES + j], row[_P["cand_ability_p0"] + j] = a, p
        if mon.item is None:
            for j, (i, p) in enumerate(items[:K_ITEMS]):
                ids[I_CAND_ITEMS + j], row[_P["cand_item_p0"] + j] = i, p
        if tera:
            row[_P["tera_p0"] + tera] = 1
        else:
            for t, p in teras:
                row[_P["tera_p0"] + t] = p
        mult = dex.item_type_mult[ids[I_ITEM]].copy()
        if ability:
            mult *= dex.ability_type_mult[ability]
        elif abilities:
            total = sum(p for _, p in abilities)
            mult *= sum(p * dex.ability_type_mult[a] for a, p in abilities) / total
        row[span("type_mult0", 20)] = mult
        if len(self._foe_cache) >= CACHE_LIMIT:
            self._foe_cache.clear()
        cached = self._foe_cache[key] = (ids, row[N_DYNAMIC:].copy(), float(stats[0]))
        return cached

    # ---- per step -------------------------------------------------------------------------------------------
    def encode(self, row: int, observation: dict) -> None:
        tracker = self.trackers[row]
        if tracker.battle_id != observation["battle_id"]:
            tracker.reset(observation["battle_id"])
        tracker.feed(observation["log"])
        request = json.loads(observation["request"]) if observation["request"] else {}
        b = self.buffers
        dyn_rows, static_rows, id_rows = [], [], []
        turn = tracker.turn
        own_side = tracker.own
        foe_side = "p2" if own_side == "p1" else "p1"
        actives = request.get("active") or []
        own_names = []
        own_fainted = 0
        team = request.get("side", {}).get("pokemon", [])[:6]
        for index, pokemon in enumerate(team):
            name = pokemon["ident"].split(": ", 1)[1]
            own_names.append(name)
            static_ids, static, estimated_hp = self._own_static(pokemon)
            hp, max_hp, status = condition(pokemon.get("condition", "0 fnt"))
            if status == "fnt":
                own_fainted += 1
                max_hp = estimated_hp
            dyn = _ZERO_DYNAMIC.copy()
            dyn[D_PRESENT] = dyn[D_OWN] = 1.0
            is_active = index < 2 and bool(pokemon.get("active"))
            self._dynamic(dyn, hp / max_hp, status, tracker.mons.get((own_side, name)), turn, is_active, index)
            dyn[D_STAT_HP] = max_hp / STAT_SCALE
            dyn[D_TERASTALLIZED] = float(bool(pokemon.get("terastallized")))
            moves = pokemon.get("moves", ())[:4]
            for m in range(len(moves)):
                dyn[D_PP0 + m] = 1.0
            if is_active and index < len(actives) and actives[index]:
                active = actives[index]
                dyn[D_TERA_AVAILABLE] = float(bool(active.get("canTerastallize")))
                dyn[D_TRAPPED] = float(bool(active.get("trapped") or active.get("maybeTrapped")))
                by_id = {m.get("id"): m for m in active.get("moves", ())}
                for m, move in enumerate(moves):
                    info = by_id.get(move)
                    if info is not None:
                        dyn[D_PP0 + m] = info.get("pp", 0) / max(info.get("maxpp", 1), 1)
                        dyn[D_DISABLED0 + m] = float(bool(info.get("disabled")))
            dyn_rows.append(dyn)
            static_rows.append(static)
            id_rows.append(static_ids)
        for index in range(len(team), 6):
            dyn_rows.append(_ZERO_DYNAMIC)
            static_rows.append(_ZERO_STATIC)
            id_rows.append(_ZERO_IDS)
        self.own_names[row] = own_names

        # Foe tokens: active a, active b, then the rest in reveal order.
        foe_keys: list = [tracker.active.get(foe_side + "a"), tracker.active.get(foe_side + "b")]
        foe_keys += [k for k in tracker.foe_order if k not in foe_keys][:4]
        foe_names = [None] * 6
        foe_fainted = revealed = 0
        foe_tera_available = float(not tracker.tera_used[foe_side])
        for k in range(6):
            key = foe_keys[k] if k < len(foe_keys) else None
            mon = tracker.mons.get(key) if key else None
            if mon is None or not mon.species:
                dyn_rows.append(_ZERO_DYNAMIC)
                static_rows.append(_ZERO_STATIC)
                id_rows.append(_ZERO_IDS)
                continue
            revealed += 1
            foe_names[k] = key[1]
            if mon.status == "fnt":
                foe_fainted += 1
            static_ids, static, max_hp = self._foe_static(mon)
            dyn = _ZERO_DYNAMIC.copy()
            dyn[D_PRESENT] = 1.0
            self._dynamic(dyn, mon.hp, mon.status, mon, turn, k < 2, k)
            dyn[D_STAT_HP] = max_hp / STAT_SCALE
            dyn[D_TERASTALLIZED] = float(mon.terastallized)
            dyn[D_TERA_AVAILABLE] = foe_tera_available
            dyn_rows.append(dyn)
            static_rows.append(static)
            id_rows.append(static_ids)
        self.foe_names[row] = foe_names
        b.floats[row, :, :N_DYNAMIC] = dyn_rows
        b.floats[row, :, N_DYNAMIC:] = static_rows
        b.ids[row] = id_rows

        f = [0.0] * N_FIELD_FLOATS
        weather, since = tracker.weather
        if weather in WEATHERS:
            f[WEATHERS[weather]] = 1.0
            if weather in EXTREME_WEATHER:
                f[FF["extreme_weather"]] = 1.0
            f[FF["weather_turns"]] = min(turn - since, 8) / 8
        terrain, since = tracker.terrain
        if terrain:
            f[FF[terrain]] = 1.0
            f[FF["terrain_turns"]] = min(turn - since, 8) / 8
        if tracker.trickroom >= 0:
            f[FF["trickroom"]] = 1.0
            f[FF["trickroom_turns"]] = min(turn - tracker.trickroom, 5) / 5
        f[FF["gravity"]] = float(tracker.gravity)
        for prefix, side in (("own_", own_side), ("foe_", foe_side)):
            for name, (start, layers) in tracker.sides[side].items():
                if name in SIDE_CONDITIONS:
                    f[FF[prefix + name]] = layers / 3 if name == "spikes" else (layers / 2 if name == "toxicspikes" else 1.0)
                    if name in TIMED_SIDE:
                        f[FF[prefix + name + "_turns"]] = min(turn - start, 8) / 8
        f[FF["turn"]] = min(turn, 60) / 30
        f[FF["own_tera_used"]] = float(tracker.tera_used[own_side])
        f[FF["foe_tera_used"]] = float(tracker.tera_used[foe_side])
        f[FF["request_wait"]] = float(bool(request.get("wait")))
        f[FF["request_switch"]] = float(bool(request.get("forceSwitch")))
        f[FF["request_move"]] = float(bool(actives))
        f[FF["own_alive"]] = (6 - own_fainted) / 6
        f[FF["foe_fainted"]] = foe_fainted / 6
        f[FF["foe_revealed"]] = revealed / 6
        f[FF["foe_unrevealed"]] = (6 - revealed) / 6
        b.field[row] = f
        b.fainted[row] = (own_fainted, foe_fainted)

    @staticmethod
    def _dynamic(dyn, hp, status, mon, turn, is_active, slot):
        alive = status != "fnt"
        dyn[D_HP] = hp
        if not alive:
            dyn[D_FAINTED] = 1.0
            return
        index = _STATUS_INDEX.get(status)
        if index is not None:
            dyn[index] = 1.0
        if not is_active:
            dyn[D_BENCH] = 1.0
            return
        dyn[D_ACTIVE_A if slot == 0 else D_ACTIVE_B] = 1.0
        if mon is None:
            return
        for stat, value in mon.boosts.items():
            index = _BOOST_INDEX.get(stat)
            if index is not None:
                dyn[index] = value / 6
        dyn[D_SWITCH_TURNS] = min(turn - mon.switch_turn, 5) / 5
        dyn[D_FRESH] = float(mon.moves_since_switch == 0)
        dyn[D_PROTECT_LAST] = float(mon.protect_turn >= turn - 1)
        if mon.volatiles:
            for name in mon.volatiles:
                index = _VOLATILE_INDEX.get(name)
                if index is not None:
                    dyn[index] = 1.0
            if "perish" in mon.volatiles:
                dyn[D_PERISH] = (mon.volatiles["perish"] + 1) / 4
