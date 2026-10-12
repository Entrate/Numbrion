"""Bots for testing the play server: random choices plus checks of the ``training.player`` event-order contract.

``RandomBot`` (factory ``play.testbots:RandomBot``) picks random moves, targets, switches and Tera from the request
and checks, at every ``set_request``, that the lines fed so far lead to that request:

* a new (not re-sent) request comes after at least one new progress line,
* the active Pokemon of the request are the ones the fed lines switched in last,
* a move request comes right after ``|turn|``.

Violations are logged and counted (``describe()["violations"]``). Options (config entry keys): ``seed``,
``switch`` (probability, 0.1), ``tera`` (0.15), ``chaos`` (probability of sending an invalid choice, to exercise
the rejection path, 0), ``delay`` ([min, max] seconds of sleep in ``decide``, [0, 0]).
"""
from __future__ import annotations

import json
import logging
import random
import threading
import time

from play.driver import NON_PROGRESS, line_type
from training.player import Bot, Decision, Player

log = logging.getLogger("play")
FOE_TARGETS = {"normal", "any", "adjacentFoe"}


def random_choice(request: dict, rng: random.Random, switch_p: float, tera_p: float) -> str | None:
    if request.get("wait"):
        return None
    if request.get("teamPreview"):
        return "default"
    mons = request["side"]["pokemon"]
    bench = [i + 1 for i, mon in enumerate(mons)
             if not mon.get("active") and not mon["condition"].endswith(" fnt")]
    used: set[int] = set()
    parts = []
    if "forceSwitch" in request:
        for need in request["forceSwitch"]:
            options = [i for i in bench if i not in used]
            if need and options:
                pick = rng.choice(options)
                used.add(pick)
                parts.append(f"switch {pick}")
            else:
                parts.append("pass")
        return ", ".join(parts)
    tera_done = False
    for slot, active in enumerate(request.get("active", [])):
        mon = mons[slot] if slot < len(mons) else {}
        if mon.get("condition", "").endswith(" fnt") or active.get("commanding"):
            parts.append("pass")
            continue
        options = [i for i in bench if i not in used]
        trapped = active.get("trapped") or active.get("maybeTrapped")
        if options and not trapped and rng.random() < switch_p:
            pick = rng.choice(options)
            used.add(pick)
            parts.append(f"switch {pick}")
            continue
        moves = [(i + 1, move) for i, move in enumerate(active.get("moves", [])) if not move.get("disabled")]
        if not moves:
            parts.append("move 1")
            continue
        index, move = rng.choice(moves)
        text = f"move {index}"
        target = move.get("target")
        if target in FOE_TARGETS:
            text += f" {rng.choice([1, 2])}"
        elif target == "adjacentAlly":
            text += f" {-2 if slot == 0 else -1}"
        elif target == "adjacentAllyOrSelf":
            text += f" {rng.choice([-1, -2])}"
        if active.get("canTerastallize") and not tera_done and rng.random() < tera_p:
            text += " terastallize"
            tera_done = True
        parts.append(text)
    return ", ".join(parts)


class RandomPlayer(Player):
    def __init__(self, bot: "RandomBot", room: str):
        super().__init__()
        self.bot, self.room = bot, room
        self.rng = random.Random(f"{bot.seed}:{room}")
        self.progress_since_request = 0
        self.last_progress = ""
        self.active: dict[str, str] = {}  # "p1a" -> name, from the fed lines
        self.requests = 0
        self.request: dict = {}
        self.closed = False

    def _violation(self, text: str) -> None:
        self.bot.count("violations")
        log.error("contract violation in %s (%s): %s", self.room, self.bot.name, text)

    def feed(self, lines: list[str]) -> None:
        if self.closed:
            self._violation("feed after close")
        super().feed(lines)
        for line in lines:
            kind = line_type(line)
            parts = line.split("|")
            if kind in ("switch", "drag", "replace") and len(parts) > 2 and ": " in parts[2]:
                position, name = parts[2].split(": ", 1)
                self.active[position[:3]] = name
            elif kind == "swap" and len(parts) > 2 and ": " in parts[2]:
                side = parts[2][:2]
                self.active[side + "a"], self.active[side + "b"] = (self.active.get(side + "b"),
                                                                    self.active.get(side + "a"))
            if kind not in NON_PROGRESS:
                self.progress_since_request += 1
                self.last_progress = line

    def set_request(self, request_json: str) -> None:
        super().set_request(request_json)
        request = json.loads(request_json)
        self.request = request
        self.bot.count("requests")
        side = request.get("side", {}).get("id")
        if side and self.side is None:
            self.side = 0 if side == "p1" else 1
        if not request.get("update"):
            if self.progress_since_request == 0:
                self._violation(f"request rqid {request.get('rqid')} without new battle lines")
            if "active" in request and not request.get("wait") and not self.last_progress.startswith("|turn|"):
                self._violation(f"move request rqid {request.get('rqid')} after {self.last_progress!r}, not |turn|")
            for slot, mon in enumerate(m for m in request.get("side", {}).get("pokemon", []) if m.get("active")):
                name = mon["ident"].split(": ", 1)[1]
                seen = self.active.get(f"{side}{'abc'[slot]}")
                if seen != name and "Zoroark" not in name and "Zorua" not in name:
                    self._violation(f"rqid {request.get('rqid')}: active {side}{'abc'[slot]} is {name} in the "
                                    f"request but {seen} in the lines")
        self.progress_since_request = 0
        self.requests += 1

    def rejected(self, message: str) -> None:
        self.bot.count("rejected")

    def decide(self) -> Decision | None:
        low, high = self.bot.delay
        if high > 0:
            time.sleep(self.rng.uniform(low, high))
        with self.bot.lock:
            self.bot.deciding += 1
            if self.bot.deciding > 1:
                self._violation("two decide() calls of one bot at the same time")
        try:
            choice = random_choice(self.request, self.rng, self.bot.switch, self.bot.tera)
            if choice is None:
                return None
            if self.rng.random() < self.bot.chaos:
                self.bot.count("invalid_sent")
                return Decision("move 9", analysis={"source": "default", "note": "deliberately invalid"})
            return Decision(choice, analysis={"source": "default", "note": "random test choice"})
        finally:
            with self.bot.lock:
                self.bot.deciding -= 1

    def close(self) -> None:
        self.closed = True


class RandomBot(Bot):
    def __init__(self, name: str = "RandomBot", seed: int = 0, switch: float = 0.1, tera: float = 0.15,
                 chaos: float = 0.0, delay=(0.0, 0.0), **_):
        self.name, self.seed = name, seed
        self.switch, self.tera, self.chaos = switch, tera, chaos
        self.delay = tuple(delay)
        self.lock = threading.Lock()
        self.deciding = 0
        self.counters: dict[str, int] = {}

    def count(self, key: str) -> None:
        with self.lock:
            self.counters[key] = self.counters.get(key, 0) + 1

    def new_player(self, room: str) -> Player:
        return RandomPlayer(self, room)

    def describe(self) -> dict:
        return {"name": self.name, "factory": "play.testbots:RandomBot", "chaos": self.chaos,
                "delay": list(self.delay), **self.counters}
