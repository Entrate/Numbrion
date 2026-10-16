"""Sleep exploration for label generation (shallow_labels.py ``--sleep-explore``) and sleep counting.

Self-play label games almost never contain sleep (the search rarely picks sleep moves), so the value head never
sees what sleep costs. ``SleepMoves.options`` finds, for one searched decision, the legal sleep-move codes of the
acting side that have an EFFECTIVE target; training.search_opponent then plays such a code with probability P
(``OpponentSearch.act``), while the decision is still labelled with the search's own strategy.

Effective target (conservative: a check that could be wrong errs towards "not effective"). The target's state is
read from the opponent's own observation row (``row ^ 1``: its request, so the true ability, item and status, plus
the public volatiles its tracker sees); field state from the same row. This only selects exploration moves; the
labels and the search are unchanged. A target is NOT effective when:

* it is not an active, living Pokemon in that slot;
* it has a status (any of brn par slp frz psn tox); Yawn also: it is already drowsy;
* Sleep Clause: any living Pokemon of its side is asleep (Rest included, to stay conservative);
* Substitute, unless the move is sound-based (Sing, Grass Whistle, Relic Song); Safeguard on its side;
* Electric or Misty Terrain and it is grounded (grounded unless Flying / Levitate / Air Balloon, and always under
  Gravity or with Iron Ball; Magnet Rise and Telekinesis are not tracked, so such targets count as grounded);
* its ability is Insomnia, Vital Spirit, Sweet Veil, Comatose, Purifying Salt or Shields Down; Sweet Veil on its
  ally; Flower Veil on it or its ally while it is Grass-type; Leaf Guard in sun (no Utility Umbrella);
* it holds a Chesto or Lum Berry (it would wake at once);
* it is or may be transformed (Ditto, Imposter, or it knows Transform): the encoder does not track Transform, so
  its current typing, ability and item checks are unreliable (an Imposter Ditto copying Breloom is Grass-type);
* Hydration while rain (or heavy rain) is up, whatever Utility Umbrella or weather suppression says: it would be
  cured at the end of the turn;
* status moves: Magic Bounce or Good as Gold; Prankster user against a Dark-type target, against a side with
  Armor Tail / Dazzling / Queenly Majesty, or against a grounded target in Psychic Terrain;
* powder moves (Spore, Sleep Powder): Grass type (Tera type when terastallized), Overcoat, Safety Goggles;
* sound moves: Soundproof;
* damaging moves with a sleep chance (Relic Song): Ghost type for a Normal move, Shield Dust, Covert Cloak.

Spread moves (Dark Void, Relic Song) are effective when one of the foe actives is. Moves missing from the dex (not
in the format's sets: Lovely Kiss, Sing, Grass Whistle, Dark Void) are accepted in the list and never match.

``SleepCounter`` counts sleep onsets in the actors' own-team tokens (status from each side's own request), keyed
by species within a row so switches that reorder the request do not count: a Pokemon that becomes asleep while
its HP jumps to full from below is counted as a Rest, every other new sleep as inflicted.
"""
from __future__ import annotations

import numpy as np

from .dex import STATUSES, Dex
from .encoder import FF, I_ABILITY, I_ITEM, I_MOVE0, I_SPECIES, I_TERA, I_TYPE1, I_TYPE2, PF

DEFAULT_SLEEP_MOVES = ("spore", "sleeppowder", "hypnosis", "yawn", "lovelykiss", "sing", "grasswhistle", "darkvoid",
                       "relicsong")
SPREAD_TARGETS = ("allAdjacentFoes", "allAdjacent")

_STATUS_COLUMNS = [PF[s] for s in STATUSES]
_SLP = PF["slp"]
_ACTIVE = (PF["active_a"], PF["active_b"])


def _ids(table: dict, names) -> set:
    return {table[n] for n in names if n in table}


class SleepMoves:
    """Sleep-move lookup tables and the effective-target check (module docstring)."""

    def __init__(self, dex: Dex, names=DEFAULT_SLEEP_MOVES):
        self.dex = dex
        self.names = tuple(names)
        self.kind = np.full(dex.n_moves, -1, dtype=np.int64)  # move index -> row of ``props`` (-1: no sleep move)
        self.props = []
        for name in self.names:
            index = dex.move_index.get(name)
            if index is None or self.kind[index] >= 0:
                continue
            move = dex.moves[index - 1]
            flags = set(move.get("flags") or ())
            self.kind[index] = len(self.props)
            self.props.append(dict(
                name=name, powder="powder" in flags, sound="sound" in flags,
                yawn=move.get("volatileStatus") == "yawn", status=move.get("category") == "Status",
                spread=move.get("target") in SPREAD_TARGETS, type=dex.type_index.get(move.get("type"), 0)))
        self.known = tuple(p["name"] for p in self.props)
        a, i, t = dex.ability_index, dex.item_index, dex.type_index
        self.immune = _ids(a, ("insomnia", "vitalspirit", "sweetveil", "comatose", "purifyingsalt", "shieldsdown"))
        self.ally_immune = _ids(a, ("sweetveil",))
        self.flower_veil = _ids(a, ("flowerveil",))
        self.leaf_guard = _ids(a, ("leafguard",))
        self.reflect_status = _ids(a, ("magicbounce", "goodasgold"))
        self.prankster = _ids(a, ("prankster",))
        self.priority_block = _ids(a, ("armortail", "dazzling", "queenlymajesty"))
        self.overcoat = _ids(a, ("overcoat",))
        self.soundproof = _ids(a, ("soundproof",))
        self.levitate = _ids(a, ("levitate",))
        self.shield_dust = _ids(a, ("shielddust",))
        self.berries = _ids(i, ("chestoberry", "lumberry"))
        self.goggles = _ids(i, ("safetygoggles",))
        self.balloon = _ids(i, ("airballoon",))
        self.iron_ball = _ids(i, ("ironball",))
        self.umbrella = _ids(i, ("utilityumbrella",))
        self.cloak = _ids(i, ("covertcloak",))
        self.grass, self.flying, self.dark, self.ghost, self.normal = (t.get(n, -1) for n in (
            "Grass", "Flying", "Dark", "Ghost", "Normal"))
        self.stellar = t.get("Stellar", -1)
        self.hydration = _ids(a, ("hydration",))
        self.imposter = _ids(a, ("imposter",))
        self.transformers = _ids(dex.species_index, ("ditto",))
        self.transform_moves = np.array(sorted(_ids(dex.move_index, ("transform",))) or [-1], dtype=np.int64)

    def has_sleep_move(self, ids: np.ndarray, rows: np.ndarray) -> np.ndarray:
        """[len(rows)] whether an own active token (0 / 1) of each row knows a sleep move (a cheap prefilter)."""
        moves = ids[rows, 0:2, I_MOVE0:I_MOVE0 + 4]
        return (self.kind[moves] >= 0).any((1, 2))

    # ---- one target ------------------------------------------------------------------------------------------
    def _types(self, ids, floats, row, token) -> set:
        tera = int(ids[row, token, I_TERA])
        if floats[row, token, PF["terastallized"]] > 0.5 and tera and tera != self.stellar:
            return {tera}
        return {int(ids[row, token, I_TYPE1]), int(ids[row, token, I_TYPE2])} - {0}

    def _may_transform(self, ids, row, token) -> bool:
        """A target that is or may be transformed: its typing, ability and moves are not what the tokens say (the
        encoder does not track Transform). Ditto, Imposter (base or current) or a known Transform."""
        return (int(ids[row, token, I_SPECIES]) in self.transformers or int(ids[row, token, I_ABILITY]) in self.imposter
                or bool(np.isin(ids[row, token, I_MOVE0:I_MOVE0 + 4], self.transform_moves).any()))

    def _grounded(self, ids, floats, field, row, token, types) -> bool:
        item, ability = int(ids[row, token, I_ITEM]), int(ids[row, token, I_ABILITY])
        if field[row, FF["gravity"]] > 0 or item in self.iron_ball:
            return True
        return not (self.flying in types or ability in self.levitate or item in self.balloon)

    def effective(self, ids, floats, field, foe: int, target: int, props: dict, user_ability: int) -> bool:
        """Whether the sleep move ``props`` would put the opponent's active ``target`` (0 / 1: its slot a / b; tokens
        of its own row ``foe``) to sleep, by the module docstring's conservative rules."""
        f = floats[foe]
        if f[target, _ACTIVE[target]] < 0.5 or f[target, PF["fainted"]] > 0.5 or f[target, PF["hp"]] <= 0:
            return False
        if (f[target, _STATUS_COLUMNS] > 0.5).any():
            return False
        if props["yawn"] and f[target, PF["yawn"]] > 0.5:
            return False
        team = f[:6]
        if ((team[:, _SLP] > 0.5) & (team[:, PF["fainted"]] < 0.5)).any():  # Sleep Clause
            return False
        if f[target, PF["substitute"]] > 0.5 and not props["sound"]:
            return False
        if field[foe, FF["own_safeguard"]] > 0:
            return False
        ability, item = int(ids[foe, target, I_ABILITY]), int(ids[foe, target, I_ITEM])
        ally = 1 - target
        ally_alive = f[ally, _ACTIVE[ally]] > 0.5 and f[ally, PF["fainted"]] < 0.5
        ally_ability = int(ids[foe, ally, I_ABILITY]) if ally_alive else 0
        types = self._types(ids, floats, foe, target)
        grounded = self._grounded(ids, floats, field, foe, target, types)
        if grounded and (field[foe, FF["electricterrain"]] > 0 or field[foe, FF["mistyterrain"]] > 0):
            return False
        if ability in self.immune or ally_ability in self.ally_immune or item in self.berries:
            return False
        if self._may_transform(ids, foe, target):
            return False
        if ability in self.hydration and field[foe, FF["rain"]] > 0:
            return False
        if self.grass in types and (ability in self.flower_veil or ally_ability in self.flower_veil):
            return False
        if ability in self.leaf_guard and field[foe, FF["sun"]] > 0 and item not in self.umbrella:
            return False
        if props["status"]:
            if ability in self.reflect_status:
                return False
            if user_ability in self.prankster and (
                    self.dark in types or ability in self.priority_block or ally_ability in self.priority_block
                    or (grounded and field[foe, FF["psychicterrain"]] > 0)):
                return False
        else:  # a damaging move whose sleep is a secondary effect
            if (props["type"] == self.normal and self.ghost in types) or ability in self.shield_dust \
                    or item in self.cloak:
                return False
        if props["powder"] and (self.grass in types or ability in self.overcoat or item in self.goggles):
            return False
        if props["sound"] and ability in self.soundproof:
            return False
        return True

    # ---- one decision ----------------------------------------------------------------------------------------
    def options(self, arrays: dict, row: int, masks: tuple) -> list[list[int]]:
        """Per slot (0 / 1), the legal sleep-move codes of row ``row`` (the acting side's own row; ``row ^ 1`` is the
        opponent's) with an effective target. ``masks``: the exact (mask0 [47], joint [47, 47]) of ``Battle.legal_mask``.
        Only Tera-free codes; single-target moves aimed at a foe slot (target codes 3 / 4), spread moves at code 2."""
        ids, floats, field = arrays["ids"], arrays["floats"], arrays["field"]
        joint = masks[1]
        legal = (joint.any(1), joint.any(0))  # slot a / slot b codes that have a legal partner
        foe = row ^ 1
        out: list[list[int]] = [[], []]
        for slot in (0, 1):
            if floats[row, slot, _ACTIVE[slot]] < 0.5:
                continue
            user_ability = int(ids[row, slot, I_ABILITY])
            for m in range(4):
                kind = int(self.kind[ids[row, slot, I_MOVE0 + m]])
                if kind < 0:
                    continue
                props = self.props[kind]
                if props["spread"]:
                    code = m * 10 + 2 * 2
                    if legal[slot][code] and any(self.effective(ids, floats, field, foe, t, props, user_ability)
                                                 for t in (0, 1)):
                        out[slot].append(code)
                    continue
                for t in (0, 1):  # foe slot a / b: target codes 3 / 4 (Showdown locations +1 / +2)
                    code = m * 10 + (3 + t) * 2
                    if legal[slot][code] and self.effective(ids, floats, field, foe, t, props, user_ability):
                        out[slot].append(code)
        return out


class SleepCounter:
    """Sleep onsets per actor row (module docstring). ``update(floats, ids, done_rows)`` after every observation;
    ``done_rows`` marks rows whose battle was just replaced (their tokens belong to a new battle)."""

    def __init__(self, rows: int):
        self.species = np.zeros((rows, 6), dtype=np.int64)
        self.asleep = np.zeros((rows, 6), dtype=bool)
        self.hp = np.zeros((rows, 6), dtype=np.float32)

    def update(self, floats: np.ndarray, ids: np.ndarray, done_rows: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
        """Returns per row the inflicted sleep onsets and the Rest-like ones since the last call."""
        species = ids[:, 0:6, I_SPECIES]
        asleep = (floats[:, 0:6, _SLP] > 0.5) & (floats[:, 0:6, PF["fainted"]] < 0.5)
        hp = floats[:, 0:6, PF["hp"]]
        same = (species[:, :, None] == self.species[:, None, :]) & (species[:, :, None] > 0)
        was_asleep = (same & self.asleep[:, None, :]).any(2)
        had_hp = np.where(same.any(2), (same * self.hp[:, None, :]).sum(2), 1.0)
        new = asleep & ~was_asleep & same.any(2) & ~done_rows[:, None]
        rest = new & (hp >= 0.999) & (had_hp < 0.999)
        self.species, self.asleep, self.hp = species.copy(), asleep, hp.copy()
        return (new & ~rest).sum(1), rest.sum(1)
