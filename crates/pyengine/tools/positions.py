"""Decision positions: a battle (packed teams + battle seed) and the choices that lead to a decision point, replayed
through a real ``numbrion.Battle`` and a protocol-driven player.

A position (one JSON line in a ``--positions-out`` file of search_eval.py)::

    {"teams": [p1, p2], "seed": [a, b, c, d], "names": null, "side": 0, "turn": 7,
     "history": [{"side": 0, "a0": 3, "a1": 41}, {"side": 1, "choice": "default"}, ...], ...}

``history`` lists every accepted choice from the start, in the order they were submitted (side 0 before side 1 at a
boundary where both act). A step has action codes ``a0``/``a1`` (replayed with ``choose_action``) or a Showdown
``choice`` text (replayed with ``choose``; ``"default"`` lets the engine choose), and optionally ``rejected``: code
pairs the engine refused before the accepted choice (replayed too, since a refusal can change the request).

Replaying feeds the player exactly what search_eval's play_game feeds it (player-view lines from
``drain_player_log``, the request before each of its choices), but answers the player's past requests with
``record_choice`` (as the advisor follows a human) instead of ``decide``. With ``warm_builder`` the world builder
processes each observation as it arrives (``WorldBuilder.update``), as it would in a live game where it builds
worlds every turn, so the decision at the end does not pay for the whole history.
"""
from __future__ import annotations

import inspect
import json
from pathlib import Path

import numpy as np

import numbrion


class ReplayError(RuntimeError):
    """A script or history does not replay: a choice is illegal, or a side is asked to act when it has no request."""


def derive_seed(*parts) -> int:
    """A deterministic 62-bit seed from integers (e.g. base seed, pair index, battle side)."""
    words = [int(p) & 0xFFFFFFFF for p in parts]
    return int(np.random.SeedSequence(words).generate_state(1, np.uint64)[0] >> np.uint64(2))


def new_player(bot, room: str, seed: int | None = None):
    """``bot.new_player(room, seed=seed)`` when the bot supports per-player seeds (SearchBot after the search2 core
    change), else ``bot.new_player(room)``. Returns (player, seeded)."""
    try:
        params = inspect.signature(bot.new_player).parameters
    except (TypeError, ValueError):
        params = {}
    if seed is not None and "seed" in params:
        return bot.new_player(room, seed=seed), True
    return bot.new_player(room), False


def step_text(battle, step: dict) -> str:
    """The Showdown choice text of a history step, computed on ``battle`` before the step is applied."""
    if step.get("a0") is not None:
        text = battle.action_text(step["side"], int(step["a0"]), int(step["a1"]))
        if text is None:
            raise ReplayError(f"step {step}: codes ({step['a0']}, {step['a1']}) are not legal for side {step['side']}")
        return text
    return step.get("choice") or "default"


class Replay:
    """A battle stepped through a list of choices, optionally feeding one side's protocol-driven player."""

    def __init__(self, teams, seed, names=None):
        self.teams = (teams[0], teams[1])
        self.seed = tuple(int(x) for x in seed)
        self.battle = numbrion.Battle(self.seed, self.teams[0], self.teams[1],
                                      names=tuple(names) if names else None, log=True)
        self.battle.start()
        self.player = None
        self.side = None
        self.warm_builder = False
        self.steps = 0
        self.log: list[str] = []  # omniscient lines so far (for scenario checks)
        # The engine lets a side change its choice until the turn runs, so a side scripted twice at one boundary is
        # caught here: the sides that must act at the current boundary, and those that have chosen.
        self._needing: set | None = None
        self._chosen: set = set()
        self._group = None  # the ``boundary`` label of the steps being applied (scenario scripts)
        self._drain_omni()

    def _drain_omni(self):
        self.log.extend(self.battle.drain_log())

    def attach(self, side: int, player, warm_builder: bool = True) -> None:
        """Feed ``player`` (side ``side``) from now on; must be called before the first step."""
        self.player, self.side, self.warm_builder = player, side, warm_builder
        player.team = self.teams[side]

    def _feed(self) -> None:
        for s in (0, 1):
            lines = self.battle.drain_player_log(s)
            if self.player is not None and s == self.side:
                self.player.feed(lines)

    def step(self, step: dict) -> None:
        battle = self.battle
        side = int(step["side"])
        where = f"step {self.steps} (turn {battle.turn}, side {side})"
        if battle.ended:
            raise ReplayError(f"{where}: the battle already ended")
        group = step.get("boundary")
        if group is not None:  # scripted boundaries must line up with the battle's
            if self._needing is not None and group != self._group:
                raise ReplayError(f"{where}: script boundary {self._group} is not finished: side(s) "
                                  f"{sorted(self._needing - self._chosen)} must act there too")
            if self._needing is None and group == self._group:
                raise ReplayError(f"{where}: side {side} has no decision to make at script boundary {group} "
                                  f"(the battle already moved on)")
            self._group = group
        if self._needing is None:
            self._needing, self._chosen = {s for s in (0, 1) if battle.needs_action(s)}, set()
        if side not in self._needing:
            raise ReplayError(f"{where}: side {side} has no decision to make here "
                              f"(sides to act: {sorted(self._needing)})")
        if side in self._chosen:
            raise ReplayError(f"{where}: side {side} already chose at this boundary (sides still to act: "
                              f"{sorted(self._needing - self._chosen)})")
        self._feed()
        for a0, a1 in step.get("rejected") or []:
            try:
                battle.choose_action(side, int(a0), int(a1))
            except numbrion.ChoiceError:
                continue
            raise ReplayError(f"{where}: a recorded rejection ({a0}, {a1}) was accepted")
        text = step_text(battle, step)
        if self.player is not None and side == self.side:
            self.player.set_request(battle.request_json(side))
            self.player.record_choice(text)
            builder = getattr(self.player, "builder", None)
            if self.warm_builder and builder is not None:
                try:
                    builder.update()
                except RuntimeError:
                    pass  # no world fits yet: decide() reports it
        try:
            if step.get("a0") is not None:
                battle.choose_action(side, int(step["a0"]), int(step["a1"]))
            else:
                battle.choose(side, text)
        except numbrion.ChoiceError as error:
            request = json.loads(battle.request_json(side) or "{}")
            raise ReplayError(f"{where}: choice {text!r} rejected: {error.args[0]}\n"
                              f"  options: {describe_request(request)}") from None
        self.steps += 1
        self._chosen.add(side)
        if self._chosen >= self._needing:
            self._needing = None
        self._drain_omni()

    def run(self, history) -> "Replay":
        for step in history:
            self.step(step)
        if self._needing is not None and self._group is not None:
            raise ReplayError(f"script boundary {self._group} is not finished: side(s) "
                              f"{sorted(self._needing - self._chosen)} must act there too")
        return self

    def ready(self, side: int | None = None) -> None:
        """Bring the player to the current decision: feed the remaining lines and set the request."""
        side = self.side if side is None else side
        if self.battle.ended:
            raise ReplayError("the battle ended before the target position")
        if not self.battle.needs_action(side) or (self._needing is not None and side in self._chosen):
            raise ReplayError(f"side {side} has no decision at the target (turn {self.battle.turn})")
        self._feed()
        if self.player is not None:
            self.player.set_request(self.battle.request_json(side))


def describe_request(request: dict) -> str:
    """Short text of a request's options (for error messages)."""
    if not request:
        return "no request"
    if request.get("wait"):
        return "wait"
    parts = []
    mons = request.get("side", {}).get("pokemon", [])
    for k, active in enumerate(request.get("active") or []):
        name = mons[k]["details"].split(",")[0] if k < len(mons) else f"slot {k}"
        moves = ", ".join(f"{i + 1}:{m['id']}{'' if not m.get('disabled') else '(disabled)'}"
                          for i, m in enumerate(active.get("moves", [])))
        parts.append(f"{name} [{moves}]")
    bench = [f"{i + 1}:{p['details'].split(',')[0]} {p['condition']}" for i, p in enumerate(mons) if not p.get("active")]
    kind = "switch" if request.get("forceSwitch") else "move"
    return f"{kind}: " + "; ".join(parts) + (f"; bench {', '.join(bench)}" if bench else "")


def replay_position(position: dict, bot=None, player_seed: int | None = None, warm_builder: bool = True):
    """Replay a stored position. With ``bot``: a new player for ``position['side']`` (seeded when the bot supports
    it) follows the history and is left ready to ``decide``. Returns (replay, player or None)."""
    replay = Replay(position["teams"], position["seed"], position.get("names"))
    player = None
    side = int(position["side"])
    if bot is not None:
        player, _ = new_player(bot, f"replay-{position.get('pair', '')}-{side}", player_seed)
        replay.attach(side, player, warm_builder)
    replay.run(position["history"])
    replay.ready(side)
    return replay, player


def read_positions(path) -> list[dict]:
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
