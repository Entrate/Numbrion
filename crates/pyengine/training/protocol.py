"""Protocol-only helpers: what a Showdown client can derive from its own requests and its battle room's lines.

A bot on a Showdown server sees only its side's ``|request|`` JSON and the room's protocol lines. The training stack
is built on the engine's view of the same information (``Battle.legal_mask``, ``Battle.action_text``, the packed team,
``Battle.drain_player_log``). This module rebuilds each of those from the client's inputs alone, so the policy, the
encoder and ``numbrion.WorldBuilder`` run unchanged against a live server and never read a live engine battle
(docs/design/SEARCH-PREREQUISITES.md item 1.4). Nothing here imports the engine.

* ``request_mask(request)``: the masks of docs/design/TRAINING-API.md ("Action space"), the rules of
  ``crates/engine/src/sim/choices/legal.rs`` applied to the request. Equal to ``Battle.legal_mask`` except where the
  engine uses facts the request hides (the request mask is then a superset, as a real client would try them):
  ``maybeTrapped`` (the last active Pokemon may be trapped by an unseen Shadow Tag / Arena Trap / Magnet Pull: the
  request offers switches the engine refuses) and ``maybeDisabled`` (an unseen Imprison hides ``disabled`` on the last
  active Pokemon's moves: the request offers moves the engine refuses, or, when Imprison covers every move, the
  engine allows only Struggle as code 4). A refused choice comes back from the server as ``[Unavailable choice]``
  with an updated request; its mask is then exact.
* ``choice_text(request, a0, a1)`` / ``codes_from_choice(request, text)``: codes to Showdown choice text and back.
* ``team_from_request(request, lines=None)``: the side's packed team from its first request.
* ``normalize_lines`` / ``LineNormalizer``: a server's room lines in the engine's player-view form.
"""
from __future__ import annotations

import json
import re
from functools import lru_cache

import numpy as np

from .dex import load_dex, to_id

# Action codes (docs/design/TRAINING-API.md); the tests check them against numbrion's constants.
N_ACTIONS = 47
SWITCH_BASE = 40
PASS = 46
TARGET_AUTO = 2
FORCED_MOVE = 4  # move slot 0, no target, no Tera: the single code of a locked move, Recharge or Struggle

# Target types for which the player names a target location (battle-actions.ts CHOOSABLE_TARGETS).
CHOOSABLE_TARGETS = frozenset({"normal", "any", "adjacentAlly", "adjacentAllyOrSelf", "adjacentFoe"})


def move_code(slot: int, target: int, tera: bool) -> int:
    """``slot`` 0..3, ``target`` the signed location -2..2 (0 = none)."""
    return slot * 10 + (target + 2) * 2 + int(tera)


def load_request(request) -> dict | None:
    """A request as a dict: accepts the dict, its JSON text, or ``None``/``"null"`` (no request)."""
    if request is None or isinstance(request, dict):
        return request
    text = request.decode() if isinstance(request, (bytes, bytearray)) else str(request)
    text = text.strip()
    if text.startswith("|request|"):
        text = text[len("|request|"):]
    return json.loads(text) if text and text != "null" else None


# ---- legal actions ---------------------------------------------------------------------------------------------

def _valid_target(target: str, loc: int, position: int) -> bool:
    """Battle.validTargetLoc for doubles (``TargetKind::valid_loc``); ``loc`` is non-zero."""
    source = -(position + 1)
    is_self = loc == source
    is_foe = loc > 0
    adjacent = abs(-(3 - loc) - source) <= 1 if loc > 0 else abs(loc - source) == 1
    if target == "normal":
        return adjacent
    if target == "adjacentAlly":
        return adjacent and not is_foe
    if target == "adjacentAllyOrSelf":
        return (adjacent and not is_foe) or is_self
    if target == "adjacentFoe":
        return adjacent and is_foe
    if target == "any":
        return not is_self
    return False


def target_locations(target: str | None, position: int) -> list[int]:
    """Signed target locations a move of request target type ``target`` may take from active ``position``:
    ``[0]`` (automatic) for spread, self and field moves, otherwise the valid non-zero locations."""
    if target not in CHOOSABLE_TARGETS:
        return [0]
    return [loc for loc in (-2, -1, 1, 2) if _valid_target(target, loc, position)]


def fainted(pokemon: dict) -> bool:
    return pokemon.get("condition", "").endswith("fnt")


class SlotOptions:
    """The codes one active slot may take on its own (``LegalSlot``) and the request's hidden-information flags."""

    __slots__ = ("codes", "auto", "revival", "forced", "maybe_trapped", "maybe_disabled")

    def __init__(self, codes, auto=False, revival=False, forced=False):
        self.codes = sorted(codes)
        self.auto = auto  # fainted, commanding or unflagged: must pass, takes no part in the budgets
        self.revival = revival  # Revival Blessing slot of a replacement request
        self.forced = forced  # locked move, Recharge or Struggle: FORCED_MOVE is plain "move 1"
        self.maybe_trapped = False
        self.maybe_disabled = False


class Options:
    """The legal options of one request, with the joint constraints of ``legal.rs`` (``kind`` "move"/"switch")."""

    def __init__(self, request: dict):
        side = request["side"]
        self.party = side["pokemon"]
        self.bench = [i for i in range(2, len(self.party)) if not fainted(self.party[i])]
        self.forced_switches = self.forced_passes = 0
        if "forceSwitch" in request:
            self.kind = "switch"
            self.slots = self._switch_slots(request["forceSwitch"])
        else:
            self.kind = "move"
            self.slots = [self._move_slot(request.get("active") or [], k) for k in range(2)]

    def _move_slot(self, active: list, k: int) -> SlotOptions:
        mon = self.party[k] if k < len(self.party) else None
        entry = active[k] if k < len(active) else None
        if mon is None or entry is None or fainted(mon) or mon.get("commanding"):
            return SlotOptions([PASS], auto=True)
        moves = (entry.get("moves") or [])[:4]
        codes = []
        # A locked move, Recharge or Struggle is the only entry and has no PP field.
        forced = len(moves) == 1 and "pp" not in moves[0]
        enabled = [] if forced else [j for j, m in enumerate(moves) if not m.get("disabled") and m.get("pp", 1) > 0]
        if not enabled:
            # An updated request may show every move disabled: chooseMove then uses Struggle.
            forced = True
            codes.append(FORCED_MOVE)
        tera = bool(entry.get("canTerastallize")) and not forced
        for j in enabled:
            for loc in target_locations(moves[j].get("target"), k):
                codes.append(move_code(j, loc, False))
                if tera:
                    codes.append(move_code(j, loc, True))
        if not entry.get("trapped"):
            codes.extend(SWITCH_BASE + i for i in self.bench)
        slot = SlotOptions(codes, forced=forced)
        slot.maybe_trapped = bool(entry.get("maybeTrapped"))
        slot.maybe_disabled = bool(entry.get("maybeDisabled"))
        return slot

    def _switch_slots(self, flags: list) -> list[SlotOptions]:
        flagged = [k < len(flags) and bool(flags[k]) for k in range(2)]
        self.forced_switches = min(sum(flagged), len(self.bench))
        self.forced_passes = sum(flagged) - self.forced_switches
        pass_ok = [PASS] if self.forced_passes else []
        dead = [i for i, mon in enumerate(self.party) if fainted(mon)]
        slots = []
        for k in range(2):
            if not flagged[k]:
                slots.append(SlotOptions([PASS], auto=True))
            elif k < len(self.party) and self.party[k].get("reviving"):
                # Never `pass` here, as in the engine's masks (mask.rs `passes_on_revival`): Showdown accepts it with
                # no living Pokemon in reserve, then throws "Choices are done immediately after a request".
                slots.append(SlotOptions([SWITCH_BASE + i for i in dead], revival=True))
            else:
                slots.append(SlotOptions([SWITCH_BASE + i for i in self.bench] + pass_ok))
        return slots

    def joint_ok(self, a0: int, a1: int) -> bool:
        """Joint legality of two codes that are legal for their slots (``SideActions::joint_ok``)."""
        if self.kind == "move":
            if a0 < SWITCH_BASE and a1 < SWITCH_BASE and a0 & 1 and a1 & 1:
                return False  # at most one Tera per side
            return not (SWITCH_BASE <= a0 < PASS and a0 == a1)  # distinct switch-ins
        forced_left, passes_left, incoming = self.forced_switches, self.forced_passes, set()
        for slot, code in zip(self.slots, (a0, a1)):
            if slot.auto:
                continue
            if code < SWITCH_BASE:
                return False
            if code == PASS:
                if passes_left == 0:
                    return False
                passes_left -= 1
            elif slot.revival:
                forced_left = max(forced_left - 1, 0)
            else:
                if code in incoming or forced_left == 0:
                    return False
                forced_left -= 1
                incoming.add(code)
        return forced_left == 0

    def legal(self, a0: int, a1: int) -> bool:
        return a0 in self.slots[0].codes and a1 in self.slots[1].codes and self.joint_ok(a0, a1)

    def joint(self) -> np.ndarray:
        out = np.zeros((N_ACTIONS, N_ACTIONS), dtype=bool)
        for a0 in self.slots[0].codes:
            for a1 in self.slots[1].codes:
                out[a0, a1] = self.joint_ok(a0, a1)
        return out


def request_options(request) -> Options | None:
    """The options of a request, ``None`` when it asks for no choice (``wait``, team preview, no request)."""
    request = load_request(request)
    if not request or request.get("wait") or request.get("teamPreview") or "side" not in request:
        return None
    if "forceSwitch" not in request and "active" not in request:
        return None
    return Options(request)


def request_mask(request) -> tuple[np.ndarray, np.ndarray]:
    """``(mask0 bool[47], joint bool[47, 47])`` from a request alone, as ``Battle.legal_mask(side)`` returns them:
    ``joint[a0, a1]`` is a legal pair, ``mask0[a0]`` = some pair starts with ``a0``. All False when the request asks
    for no choice. See the module docstring for the cases where the engine mask is smaller."""
    options = request_options(request)
    if options is None:
        return np.zeros(N_ACTIONS, dtype=bool), np.zeros((N_ACTIONS, N_ACTIONS), dtype=bool)
    joint = options.joint()
    return joint.any(axis=1), joint


# ---- choice text -----------------------------------------------------------------------------------------------

def _slot_text(slot: SlotOptions, code: int) -> str:
    if code == PASS:
        return "pass"
    if code >= SWITCH_BASE:
        return f"switch {code - SWITCH_BASE + 1}"
    if code == FORCED_MOVE and slot.forced:
        return "move 1"
    loc = (code % 10) // 2 - 2
    return f"move {code // 10 + 1}" + (f" {loc:+d}" if loc else "") + (" terastallize" if code & 1 else "")


def choice_text(request, a0: int, a1: int) -> str | None:
    """Showdown choice text of a legal pair (``"move 1 +2 terastallize, switch 3"``), the same string as
    ``Battle.action_text``; ``None`` if the pair is not legal for the request."""
    options = request_options(request)
    if options is None or not options.legal(a0, a1):
        return None
    return f"{_slot_text(options.slots[0], a0)}, {_slot_text(options.slots[1], a1)}"


_TARGET_SUFFIX = re.compile(r"\s([-+]?[1-3])$")
_EVENTS = (" terastallize", " terastal", " mega", " megax", " megay", " zmove", " ultra", " dynamax", " gigantamax",
           " max")


def codes_from_choice(request, text: str) -> tuple[int, int]:
    """The codes of Showdown choice text for this request (the inverse of ``choice_text``, for logging what a human
    chose). Follows ``Side.choose``: comma-separated slot choices, ``move <n|name> [target] [terastallize]``,
    ``switch <n|name>``, ``pass``; slots that must pass are skipped before a non-pass choice and may be omitted at
    the end. A ``/choose `` prefix and a ``|rqid`` suffix are ignored. Any move of a forced slot is code 4. Raises
    ``ValueError`` for text it cannot map to a legal pair (``default``, ``team``, unknown moves, illegal pairs)."""
    request = load_request(request)
    options = request_options(request)
    if options is None:
        raise ValueError("the request asks for no choice")
    text = text.strip()
    if text.startswith("/choose "):
        text = text[len("/choose "):]
    text = re.sub(r"\|\d*$", "", text).strip()
    codes: list[int] = []
    for part in text.split(","):
        kind, _, data = part.strip().partition(" ")
        data = data.strip()
        if kind in ("pass", "skip"):
            if len(codes) >= 2:
                raise ValueError(f"too many choices in {text!r}")
            codes.append(PASS)
            continue
        while len(codes) < 2 and options.slots[len(codes)].auto:
            codes.append(PASS)
        if len(codes) >= 2:
            raise ValueError(f"too many choices in {text!r}")
        k = len(codes)
        if kind == "move":
            codes.append(_move_choice(request, options, k, data))
        elif kind == "switch":
            codes.append(SWITCH_BASE + _party_index(options.party, data))
        else:
            raise ValueError(f"unsupported choice {part.strip()!r}")
    while len(codes) < 2 and options.slots[len(codes)].auto:
        codes.append(PASS)
    if len(codes) != 2 or not options.legal(*codes):
        raise ValueError(f"{text!r} is not a legal choice for this request")
    return codes[0], codes[1]


def _move_choice(request: dict, options: Options, k: int, data: str) -> int:
    loc, tera = 0, False
    while True:
        m = _TARGET_SUFFIX.search(data)
        if m and to_id(data) != "conversion2":
            loc = int(m.group(1))
            data = data[:m.start()].strip()
            continue
        event = next((e for e in _EVENTS if data.endswith(e)), None)
        if event is None:
            break
        if event not in (" terastallize", " terastal"):
            raise ValueError(f"unsupported move event {event.strip()!r}")
        tera = True
        data = data[:-len(event)].strip()
    if options.kind != "move":
        raise ValueError("a replacement request takes switch or pass")
    if options.slots[k].forced:
        return FORCED_MOVE
    moves = request["active"][k]["moves"]
    if data.isdigit():
        slot = int(data) - 1
    else:
        slot = next((j for j, mv in enumerate(moves) if mv.get("id") == to_id(data)), -1)
    if not 0 <= slot < min(len(moves), 4):
        raise ValueError(f"no move {data!r}")
    return move_code(slot, loc, tera)


def _party_index(party: list, data: str) -> int:
    if data.isdigit():
        index = int(data) - 1
    else:
        index = next((i for i, mon in enumerate(party)
                      if mon["ident"].split(": ", 1)[-1].lower() == data.lower()
                      or to_id(mon["details"].split(",")[0]) == to_id(data)), -1)
    if not 0 <= index < len(party):
        raise ValueError(f"no party member {data!r}")
    return index


# ---- the own team ----------------------------------------------------------------------------------------------

STAT_NAMES = ("hp", "atk", "def", "spa", "spd", "spe")
# Gen 9 random doubles spreads (data/random-battles/gen9/teams.ts randomSet): EVs 85 and IVs 31, except Atk 0/0 for
# sets without an Atk-using move and Spe 0/0 for Trick Room / Gyro Ball sets; HP EVs may drop in steps of 4.
SPREADS = ((85, 31), (0, 0))
# Items a lead can use up before the first request (switch-in abilities, Intimidate, surge terrains).
START_CONSUMED = ("boosterenergy", "whiteherb", "mirrorherb", "electricseed", "grassyseed", "mistyseed", "psychicseed",
                  "roomservice", "adrenalineorb", "ejectpack")
DETAILS_FORMES = {("greninja", "battlebond"): "Greninja-Bond", ("rockruff", "owntempo"): "Rockruff-Dusk"}


def pack_name(name: str) -> str:
    """Showdown's ``Teams.packName``."""
    return re.sub(r"[^A-Za-z0-9]+", "", name or "")


def stat_value(base: int, ev: int, iv: int, level: int) -> int:
    """A non-HP stat without nature (``spreadModify``)."""
    return (2 * base + iv + ev // 4) * level // 100 + 5


def hp_value(base: int, ev: int, iv: int, level: int) -> int:
    return (2 * base + iv + ev // 4 + 100) * level // 100 + 10


def generator_hp_ev(base_hp: int, level: int, moves, item: str, ability: str, species_id: str) -> int:
    """The HP EV ``randomSet`` settles on in doubles (``item``/``ability`` are display names)."""
    moves = set(moves)
    ev = 85
    while ev > 1:
        hp = hp_value(base_hp, ev, 31, level)
        if ("substitute" in moves and item == "Sitrus Berry") or species_id == "minior":
            if hp % 4 == 0:
                break
        elif moves & {"bellydrum", "filletaway", "shedtail"} and (item == "Sitrus Berry" or ability == "Gluttony"):
            if hp % 2 == 0:
                break
        elif "substitute" in moves and "endeavor" in moves:
            if hp % 4 > 0:
                break
        else:
            break
        ev -= 4
    return ev


def _details(text: str) -> tuple[str, int, str, bool]:
    parts = text.split(", ")
    level, gender, shiny = 100, "", False
    for p in parts[1:]:
        if p in ("M", "F"):
            gender = p
        elif p == "shiny":
            shiny = True
        elif p[:1] == "L" and p[1:].isdigit():
            level = int(p[1:])
    return parts[0], level, gender, shiny


def _start_events(lines, side: str) -> tuple[dict[str, str], set[str]]:
    """From the lines before the first ``|turn|``: items the side's Pokemon used up (name -> item id, from
    ``|-enditem|``) and the side's Pokemon that transformed (Imposter)."""
    lost: dict[str, str] = {}
    transformed: set[str] = set()
    for line in lines:
        parts = line.split("|")
        if len(parts) > 1 and parts[1] == "turn":
            break
        if len(parts) < 4 or parts[2][:2] != side or ": " not in parts[2]:
            continue
        name = parts[2].split(": ", 1)[1]
        if parts[1] == "-enditem":
            lost.setdefault(name, to_id(parts[3]))
        elif parts[1] == "-transform":
            transformed.add(name)
    return lost, transformed


def team_from_request(request, lines=None) -> str:
    """The side's packed team (``Teams.pack`` of the generator's sets, as in the pool files) from the battle's
    **first** request, whose ``side.pokemon`` is still in team order and shows every set at full HP.

    Recovered exactly: name, species (battle-start formes undone: Zacian/Zamazenta-Crowned, a lead Terapagos-Terastal
    and its Tera Shell, Greninja-Bond whose details omit the forme), item, ability (``baseAbility``), moves, EVs/IVs
    (from the stats, the level and the base stats, using the generator's spreads and HP rule; a lead Minior-Meteor
    shows Meteor stats), gender (``N`` when the details show none), shiny, level and Tera type. Nature, happiness,
    Hidden Power type, Poke Ball, Gigantamax and Dynamax level are never set by the generator and stay empty.

    Two things a lead can lose before turn 1 are not in the request: an item used up at the start (White Herb after
    Intimidate, Booster Energy, ...) and, for an Imposter Ditto that transformed, its own moves. ``lines`` (the
    player-view lines up to the first ``|turn|``) recover the item from ``|-enditem|``; without them the most likely
    start-consumable item of the species' random-battle sets is used. A transformed Imposter user gets its set's
    moves from the random-battle sets. Raises ``ValueError`` when a stat cannot be explained by a generator spread."""
    request = load_request(request)
    dex = load_dex()
    lost, transformed = _start_events(lines, request["side"]["id"]) if lines is not None else ({}, None)
    return "]".join(_pack_set(dex, mon, index < 2, lost, transformed)
                    for index, mon in enumerate(request["side"]["pokemon"]))


def _species_entry(dex, name: str) -> dict:
    sid = to_id(name)
    entry = dex.species_by_id.get(sid) or dex.species_by_id.get(dex.aliases.get(sid, ""))
    if entry is None:
        raise ValueError(f"unknown species {name!r}")
    return entry


def _pack_set(dex, mon: dict, lead: bool, lost: dict, transformed: set | None) -> str:
    name = mon["ident"].split(": ", 1)[1]
    shown, level, gender, shiny = _details(mon["details"])
    # Details name Greninja-Bond and Rockruff-Dusk by their base species (Pokemon.getUpdatedDetails).
    shown = DETAILS_FORMES.get((to_id(shown), mon["baseAbility"]), shown)
    current = _species_entry(dex, shown)
    species = shown
    if current["battleOnly"]:
        species = dex.species_by_id[current["battleOnly"]]["name"]
    original = _species_entry(dex, species)
    # The random-battle sets' key: cosmetic formes (Minior-Blue, Gastrodon-East) share their base species' sets.
    base_id = dex.base_species(dex.aliases.get(original["id"], original["id"]))

    ability = mon["baseAbility"]
    if ability not in original["abilities"] and len(original["abilities"]) == 1:
        ability = next(iter(original["abilities"]))  # Tera Shift became Tera Shell with the forme
    moves = list(mon["moves"])  # Zacian/Zamazenta-Crowned sets already list Behemoth Blade/Bash
    copied = name in transformed if transformed is not None else (lead and ability == "imposter")
    if copied:
        pools = {tuple(s["movepool"]) for s in dex.sets.get(base_id, []) if len(s["movepool"]) <= 4}
        if len(pools) == 1:
            moves = list(next(iter(pools)))

    item = mon["item"]
    if not item and lead:
        item = lost.get(name, "")
        if not item:
            prior = (dex.species_by_id.get(base_id) or original).get("items") or {}
            usable = [(p, i) for i, p in prior.items() if i in START_CONSUMED]
            item = max(usable)[1] if usable else ""
    item_name = _names("items").get(item, item)
    ability_name = _names("abilities").get(ability, ability)

    max_hp = int(mon["condition"].split()[0].split("/")[1])
    evs, ivs = _spread(dex, current, original, base_id, level, mon["stats"], max_hp, moves, item_name, ability_name)
    ev_text = ",".join(str(evs[s]) if evs[s] else "" for s in STAT_NAMES)
    iv_text = ",".join("" if ivs[s] == 31 else str(ivs[s]) for s in STAT_NAMES)
    pokeball = "" if mon.get("pokeball", "pokeball") == "pokeball" else pack_name(mon["pokeball"])
    fields = [
        name,
        "" if pack_name(name) == pack_name(species) else pack_name(species),
        pack_name(item_name),
        pack_name(ability_name),
        ",".join(moves),
        "",  # nature
        "" if ev_text == ",,,,," else ev_text,
        gender or "N",
        "" if iv_text == ",,,,," else iv_text,
        "S" if shiny else "",
        "" if level == 100 else str(level),
        f",,{pokeball},,,{mon.get('teraType', '')}",  # happiness, then hpType, pokeball, G, dynamaxLevel, teraType
    ]
    return "|".join(fields)


@lru_cache(maxsize=None)
def _names(table: str) -> dict[str, str]:
    """Display names by id of the dex's ``items`` or ``abilities``."""
    return {entry["id"]: entry["name"] for entry in getattr(load_dex(), table)}


def _spread(dex, current, original, base_id, level, stats, max_hp, moves, item, ability):
    """EVs and IVs that reproduce the request's stats: the non-HP stats come from the current species' base stats
    (``baseStoredStats`` follow every non-transform forme change, e.g. a lead Minior-Meteor), HP from the shown
    species (max HP changes only with the details)."""
    evs, ivs = {}, {}
    family = {original["id"], base_id}
    candidates = [current] + [s for s in dex.species if s["battleOnly"] in family and s is not current]
    if original is not current:
        candidates.append(original)
    for species in candidates:
        found = {}
        for stat in STAT_NAMES[1:]:
            match = [sp for sp in SPREADS if stat_value(species["baseStats"][stat], *sp, level) == stats[stat]]
            if not match:
                break
            found[stat] = match[0]
        if len(found) == 5:
            break
    else:
        raise ValueError(f"{current['name']}: stats {stats} match no generator spread at level {level}")
    for stat, (ev, iv) in found.items():
        evs[stat], ivs[stat] = ev, iv

    base_hp = current["baseStats"]["hp"]
    ev = generator_hp_ev(original["baseStats"]["hp"], level, moves, item, ability, base_id)
    if hp_value(base_hp, ev, 31, level) != max_hp:
        fits = [e for e in range(85, 0, -4) if hp_value(base_hp, e, 31, level) == max_hp]
        if not fits:
            raise ValueError(f"{current['name']}: max HP {max_hp} matches no generator spread at level {level}")
        ev = fits[0]
    evs["hp"], ivs["hp"] = ev, 31
    return evs, ivs


# ---- server lines ----------------------------------------------------------------------------------------------

# Battle commands the engine writes to a player's stream (besides every minor `-` command).
ENGINE_COMMANDS = frozenset({
    "", "t:", "gametype", "player", "gen", "tier", "rule", "teamsize", "start", "switch", "drag", "replace",
    "detailschange", "swap", "move", "cant", "faint", "turn", "upkeep", "win", "tie", "message", "bigerror",
})
# Lines a server room adds that look like battle lines (server/room-battle.ts).
SERVER_TEXTS = (
    re.compile(r"^\|-hint\|If you want to tie earlier, consider using `/offertie`\.$"),
    re.compile(r"^\|-message\|All players are inactive\.$"),
    re.compile(r"^\|-message\|.* (forfeited|lost due to inactivity)\.$"),
    re.compile(r"^\|bigerror\|(?!You will auto-tie )"),
)


class LineNormalizer:
    """Turns the lines of a Showdown battle room, as a websocket client receives them, into the player-view lines
    of ``Battle.drain_player_log(side)``; keeps state between calls, so it can be fed one server message at a time.

    The server forwards the simulator's per-player stream (``extractChannelMessages``, as the engine does) and adds
    room lines. Rules: keep every command the engine itself writes, drop the rest:

    * dropped: ``>room`` headers, empty lines, ``|request|``, ``|error|``, room and UI lines (``|init|``, ``|title|``,
      ``|j|``/``|J|``/``|l|``/``|L|``/``|n|``/``|N|``, ``|c|``/``|c:|``, ``|:|``, ``|raw|``, ``|html|``,
      ``|uhtml|``/``|uhtmlchange|``, ``|inactive|``/``|inactiveoff|``, ``|rated|``, ``|debug|``, ...), ``||text``
      timer notes (a bare ``|`` separator is kept), and the server's own battle-looking lines (forfeit and
      inactivity ``|-message|``, the ``/offertie`` ``|-hint|``, a crash ``|bigerror|``);
    * ``|t:|<unix time>`` becomes ``|t:|`` (the engine writes no timestamp);
    * ``|player|`` is kept only for the simulator's two lines between ``|gametype|`` and ``|gen|``, as
      ``|player|pN|Name||`` (avatar and rating dropped); the room's join/leave/reconnect ``|player|`` lines are not.
    """

    def __init__(self):
        self.phase = 0  # 0 before |gametype|, 1 in the player block, 2 after |gen|

    def __call__(self, lines) -> list[str]:
        out = []
        for line in lines:
            line = line.rstrip("\r")
            if not line.startswith("|"):
                continue
            cmd = line.split("|", 2)[1]
            if cmd == "gametype":
                self.phase = 1
            elif cmd == "gen" and self.phase == 1:
                self.phase = 2
            if cmd == "t:":
                out.append("|t:|")
            elif cmd == "player":
                fields = line.split("|")
                if self.phase == 1 and len(fields) > 3 and fields[3]:
                    out.append(f"|player|{fields[2]}|{fields[3]}||")
            elif cmd == "":
                if line == "|":
                    out.append(line)
            elif (cmd in ENGINE_COMMANDS or cmd.startswith("-")) and not any(p.match(line) for p in SERVER_TEXTS):
                out.append(line)
        return out

    feed = __call__


def normalize_lines(lines) -> list[str]:
    """``LineNormalizer`` applied to one battle room's whole stream (see there for the rules). ``lines`` may be the
    raw message texts split on newlines, ``|request|`` lines included."""
    return LineNormalizer()(lines)
