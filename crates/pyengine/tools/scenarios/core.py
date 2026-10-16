"""Scenario positions for the search: teams, a battle seed and scripted choices that reach a target position, the side
to decide, and an expectation about its choice.

A scenario's script is a list of boundaries, each ``(p1 choice, p2 choice)`` in Showdown choice text (``"move 1 +1,
move 2"``, ``"switch 3"``, ``"default"``), ``None`` for a side that does not act there. Replaying it through a real
``numbrion.Battle`` must reach the target: every scripted choice legal, every side that has to act scripted, and the
``Target`` checks (turn, active Pokemon, HP and status, log lines) true. Anything else raises ``ScenarioError``.

A choice is matched as one token per slot:

* ``move:<move id>[@<target>][+tera]``, e.g. ``move:spore@+1`` (target: Showdown's location, +1/+2 foe slots a/b,
  -1/-2 own slots a/b; omitted for moves without a chosen target);
* ``switch:<species id>``;
* ``pass``.

``Expect`` patterns are ``fnmatch`` patterns on these tokens (``move:protect*``, ``switch:*``, ``*+tera``).
"""
from __future__ import annotations

import json
import re
import zlib
from dataclasses import dataclass, field
from fnmatch import fnmatch
from typing import Callable

from tools.positions import Replay, ReplayError

PASS = 46
SWITCH_BASE = 40


class ScenarioError(RuntimeError):
    """The script does not replay, or the position it reaches is not the target."""


def species_id(text: str) -> str:
    return re.sub(r"[^a-z0-9]", "", text.split(",")[0].lower())


def slot_token(request: dict, slot: int, code: int) -> str:
    """The match token of one slot's action code under ``request`` (module docstring)."""
    if code is None or code < 0:
        return "none"
    if code == PASS:
        return "pass"
    if code >= SWITCH_BASE:
        party = request.get("side", {}).get("pokemon", [])
        p = code - SWITCH_BASE
        return f"switch:{species_id(party[p]['details']) if p < len(party) else p + 1}"
    move_slot, rest = divmod(code, 10)
    loc, tera = rest // 2 - 2, rest % 2
    active = request.get("active") or []
    moves = active[slot].get("moves", []) if slot < len(active) else []
    move = moves[move_slot]["id"] if move_slot < len(moves) else f"move{move_slot + 1}"
    return f"move:{move}" + (f"@{loc:+d}" if loc else "") + ("+tera" if tera else "")


@dataclass
class ChoiceView:
    """A decided choice as the expectations see it."""
    text: str
    tokens: tuple
    readable: str = ""

    def __str__(self) -> str:
        return " | ".join(self.tokens)


@dataclass
class Expect:
    """What is acceptable for the deciding side. Every given rule must hold:

    * ``accept[slot]``: the slot's token matches one of the patterns;
    * ``reject[slot]``: the slot's token matches none of the patterns;
    * ``any_slot``: at least one slot's token matches one of the patterns;
    * ``predicate(view)``: a custom check on the ``ChoiceView``.
    """
    accept: dict = field(default_factory=dict)
    reject: dict = field(default_factory=dict)
    any_slot: list = field(default_factory=list)
    predicate: Callable | None = None
    text: str = ""

    def check(self, view: ChoiceView) -> bool:
        tokens = view.tokens
        for slot, patterns in self.accept.items():
            if not any(fnmatch(tokens[slot], p) for p in patterns):
                return False
        for slot, patterns in self.reject.items():
            if any(fnmatch(tokens[slot], p) for p in patterns):
                return False
        if self.any_slot and not any(fnmatch(t, p) for t in tokens for p in self.any_slot):
            return False
        if self.predicate is not None and not self.predicate(view):
            return False
        return True

    def describe(self) -> str:
        if self.text:
            return self.text
        parts = [f"slot {s} in {p}" for s, p in self.accept.items()]
        parts += [f"slot {s} not in {p}" for s, p in self.reject.items()]
        if self.any_slot:
            parts.append(f"a slot in {self.any_slot}")
        return "; ".join(parts) or "custom"


@dataclass
class Target:
    """Checks on the position the script reaches. ``mons`` maps (side, species id) to conditions, each one of
    ``"active"``, ``"bench"``, ``"fnt"``, a status (``"slp"``, ``"par"``, ``"brn"``, ``"psn"``, ``"tox"``,
    ``"frz"``, ``"ok"`` = no status), or an HP bound in percent (``"hp<50"``, ``"hp>=90"``). ``log``: substrings
    that must appear in the omniscient log; ``not_log``: must not. ``request``: the deciding side's request kind
    ("move" or "switch")."""
    turn: int | None = None
    mons: dict = field(default_factory=dict)
    log: list = field(default_factory=list)
    not_log: list = field(default_factory=list)
    request: str | None = "move"


@dataclass
class Scenario:
    name: str
    family: str                  # sleep, yawn, endgame, control
    teams: tuple                 # packed p1, p2
    seed: tuple                  # battle seed (four 16-bit words)
    script: list                 # [(p1 choice | None, p2 choice | None), ...]
    side: int = 0                # the side that decides at the target
    target: Target = field(default_factory=Target)
    expect: Expect | None = None  # None: informational (distribution only)
    rationale: str = ""
    names: tuple | None = None

    @property
    def informational(self) -> bool:
        return self.expect is None

    def key(self) -> int:
        return zlib.crc32(self.name.encode())

    def history(self) -> list[dict]:
        steps = []
        for k, boundary in enumerate(self.script):
            for side, text in enumerate(boundary):
                if text is not None:
                    steps.append(dict(side=side, choice=text, boundary=k))
        return steps


def mon_states(battle) -> dict:
    """{(side, species id): dict(active, hp_percent, status, fainted)} from both sides' requests (exact HP)."""
    out = {}
    for side in (0, 1):
        text = battle.request_json(side)
        request = json.loads(text) if text else {}
        for p in request.get("side", {}).get("pokemon", []):
            condition = p.get("condition", "")
            hp, _, status = condition.partition(" ")
            fainted = status == "fnt" or hp == "0"
            current, _, top = hp.partition("/")
            percent = 0.0 if fainted or not top else 100.0 * int(current) / int(top)
            out[(side, species_id(p["details"]))] = dict(active=bool(p.get("active")), hp=round(percent, 1),
                                                         status="fnt" if fainted else status, condition=condition)
    return out


def _check_condition(state: dict, condition: str) -> bool:
    if condition == "active":
        return state["active"] and state["status"] != "fnt"
    if condition == "bench":
        return not state["active"] and state["status"] != "fnt"
    if condition == "ok":
        return state["status"] == ""
    match = re.fullmatch(r"hp(<=|>=|<|>|==)(\d+(?:\.\d+)?)", condition)
    if match:
        op, value = match.group(1), float(match.group(2))
        hp = state["hp"]
        return dict(zip(("<=", ">=", "<", ">", "=="),
                        (hp <= value, hp >= value, hp < value, hp > value, hp == value)))[op]
    return state["status"] == condition


def check_target(scenario: Scenario, replay: Replay) -> list[str]:
    """Differences between the reached position and ``scenario.target`` (empty: on target)."""
    battle, target = replay.battle, scenario.target
    errors = []
    if battle.ended:
        return ["the battle ended"]
    if not battle.needs_action(scenario.side):
        return [f"side {scenario.side} has no decision (turn {battle.turn})"]
    if target.turn is not None and battle.turn != target.turn:
        errors.append(f"turn {battle.turn}, expected {target.turn}")
    if target.request is not None and battle.request_kind(scenario.side) != target.request:
        errors.append(f"request {battle.request_kind(scenario.side)}, expected {target.request}")
    states = mon_states(battle)
    for (side, species), conditions in target.mons.items():
        state = states.get((side, species))
        if state is None:
            errors.append(f"p{side + 1} has no {species}")
            continue
        for condition in ([conditions] if isinstance(conditions, str) else conditions):
            if not _check_condition(state, condition):
                errors.append(f"p{side + 1} {species}: expected {condition}, is {state['condition']}"
                              f"{' (active)' if state['active'] else ''}")
    text = "\n".join(replay.log)
    errors += [f"log lacks {s!r}" for s in target.log if s not in text]
    errors += [f"log has {s!r}" for s in target.not_log if s in text]
    return errors


def build(scenario: Scenario, player=None, warm_builder: bool = True) -> Replay:
    """Replay the script (feeding ``player`` for the deciding side when given), check the target, and leave the
    player at the decision. Raises ScenarioError."""
    try:
        replay = Replay(scenario.teams, scenario.seed, scenario.names)
    except ValueError as error:  # a team the engine does not accept
        raise ScenarioError(f"{scenario.name}: {error}") from None
    if player is not None:
        replay.attach(scenario.side, player, warm_builder)
    try:
        replay.run(scenario.history())
    except ReplayError as error:
        raise ScenarioError(f"{scenario.name}: {error}") from None
    errors = check_target(scenario, replay)
    if errors:
        raise ScenarioError(f"{scenario.name}: not at the target position: " + "; ".join(errors))
    replay.ready(scenario.side)
    return replay


def view_of(decision, request: dict) -> ChoiceView:
    tokens = (slot_token(request, 0, decision.a0), slot_token(request, 1, decision.a1))
    slots = (decision.analysis or {}).get("slots") or []
    readable = "; ".join(f"{s.get('pokemon', '')}: {s.get('text', '')}" for s in slots)
    return ChoiceView(text=decision.choice, tokens=tokens, readable=readable)


def describe_position(replay: Replay, side: int, tail: int = 25) -> str:
    """A readable summary of the position (for authoring): both sides' Pokemon, the side's options, the log tail."""
    battle = replay.battle
    lines = [f"turn {battle.turn}; needs action: {[battle.needs_action(s) for s in (0, 1)]}"]
    for (s, species), st in mon_states(battle).items():
        lines.append(f"  p{s + 1} {species:<16} {'ACTIVE' if st['active'] else '      '} {st['condition']}")
    from tools.positions import describe_request

    for s in (0, 1):
        text = battle.request_json(s)
        lines.append(f"  p{s + 1} options: {describe_request(json.loads(text) if text else {})}")
    lines.append("  log tail:")
    lines += ["    " + line for line in replay.log[-tail:] if not line.startswith("|split")]
    return "\n".join(lines)


def probe(scenario: Scenario, p1: str | None, p2: str | None, n: int = 20, seed: int = 1) -> list[dict]:
    """Play the choices ``p1``/``p2`` from the target position under ``n`` reseeds of the battle, returning each
    outcome's Pokemon states and the turn's log (for checking a scenario's rationale with the engine)."""
    import numpy as np

    base = build(scenario).battle
    rng = np.random.default_rng(seed)
    outcomes = []
    for _ in range(n):
        battle = base.clone()
        battle.reseed(tuple(int(x) for x in rng.integers(0, 1 << 16, size=4)))
        battle.drain_log()
        for side, text in ((0, p1), (1, p2)):
            if text is not None and battle.needs_action(side):
                battle.choose(side, text)
        outcomes.append(dict(states=mon_states(battle), log=battle.drain_log(), ended=battle.ended,
                             winner=battle.winner))
    return outcomes
