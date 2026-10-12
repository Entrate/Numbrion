"""One bot's side of one Showdown battle room, driven through the ``training.player`` contract.

Event order. ``Player`` wants ``feed(lines) -> set_request(json) -> decide()``, with every line that leads to a
request fed before the request. Showdown sends a side's ``|request|`` as its own websocket message, separate from
the battle lines. Which comes first depends on the server version: the pinned server (and current upstream, sim
commit "Emit requests after updates", #11105) sends the update lines first and the request right after; older
servers sent the request first and the lines right after. ``BattleRoom`` handles both:

* Every non-request line is buffered. A line counts as *progress* unless it is chat, join/leave, timestamps,
  the battle header (``|player|``, ``|gen|``, ``|rule|``, ...) or other ladder-only lines (``NON_PROGRESS``).
* A request is handed to the player once progress lines have arrived since the previous request (lines-first
  servers: immediately), or at once when it is a re-sent request (``"update": true``, or the one that follows
  ``|error|[Unavailable choice]``), which has no lines of its own.
* Otherwise the request is held until the next message with progress lines arrives (request-first servers),
  then those lines are fed first. A held request is dispatched anyway after ``HOLD_SECONDS``.

Rejected choices. ``[Invalid choice]`` errors come without a new request: the driver calls ``rejected`` and
``decide`` again on the same request. ``[Unavailable choice]`` errors are followed by a re-sent request: the
driver calls ``rejected``, waits for it, then ``set_request`` and ``decide``. After two refusals on one request
the driver sends ``default``. Errors about stale choices ("too late", "nothing to choose") are ignored.

Threads. All calls into a bot's players run on that bot's single worker thread (``BotWorker``), in submission
order, so the event loop keeps answering the websocket while a bot thinks and one bot never decides twice at once.

Deadlines (``DecisionTiming``, ladder play). Each request's time is measured from the receipt of its message. The
player gets the search allowance as ``budget_ms`` and the absolute ``deadline`` (``time.monotonic()``), and posts a
policy decision early as ``fallback`` (``(token, Decision)``). If its decision is not back by the deadline (less
``reserve_s``, and earlier when the battle clock runs low), the room sends that fallback, or ``default`` when there is
none yet, and tells the player afterwards (``record_choice``) once its late decision is done. Computed decisions are
held until ``min_response_s`` after receipt; fallbacks go out at once.

Observer events. ``observer.room_event(room, kind, data)`` is called on the event loop for every event below; it must
not block (queue the work). Exceptions are logged and ignored. ``time``/``received``/``submitted`` are ``time.time()``.

* ``message`` {time, text}: every websocket message of the room, as received.
* ``sent`` {time, text}: every command the room sends (``ROOM|/choose ...|rqid``).
* ``request`` {seq, rqid, received, text, lines, resent, clock}: a request handed to the player with the lines fed
  before it; ``seq`` counts the room's requests; ``clock`` is the last battle-clock reading or None.
* ``decision`` {seq, rqid, turn, choice, a0, a1, source, fallback, analysis, timing}: a choice sent. ``source`` is
  the analysis source (``search``, ``policy``, ``default``) or ``fallback``; ``fallback`` names why the room did not
  wait for the player (None normally). ``timing`` = {received, dispatched, decided, submitted, deadline, budget_ms,
  queue_ms, compute_ms, paced_ms, total_ms}: ``decided`` is None for a fallback, ``total_ms`` = receipt to send.
* ``late_decision`` {seq, rqid, choice, analysis, compute_ms}: the player's decision after a fallback was sent.
* ``refused`` {rqid, choice, message}; ``server_error`` {message}.
* ``timer`` {time, state ("on"/"off"), line}; ``clock`` {time, turn_left, total_left, grace, line}.
* ``rating`` {time, user, userid, old, new, delta, reason, line}: a ladder rating line after a rated battle.
* ``end`` {time, result, won, end_reason, turns}: ``won`` is True/False/None (tie or unknown side); ``end_reason`` is
  ``normal``, ``forfeit``, ``inactivity`` or ``other``.
* ``closed`` {time, ended, abandoned, stats, player_stats, rated, players, ratings}: the room is done (``ended``: the
  battle finished; ``abandoned``: connection lost or shutdown before the end).
* ``unhealthy`` {time, reason}: the bot's worker is stuck (a decision far past its deadline).
"""
from __future__ import annotations

import asyncio
import functools
import html
import json
import logging
import re
import time
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import Awaitable, Callable, Protocol

from training.player import Bot, Decision, Player

log = logging.getLogger("play")

FORMAT = "gen9randomdoublesbattle"
HOLD_SECONDS = 1.0  # longest wait for the lines of a request (or for a re-sent request) before going without
CLOCK_STALE_S = 5.0  # a "Time left" line this much older than a request belongs to an earlier one
THINK_MARGIN_MS = 250.0  # of the time left to the deadline, kept back from the player's search budget

# Line types that never lead to a request: chat and room membership, server messages, timestamps, the battle
# header, the request side channel. Everything else (|move|, |switch|, |-damage|, |turn|, |upkeep|, |start|,
# |teampreview|, |win|, ...) is a progress line.
NON_PROGRESS = frozenset({
    "", "init", "title", "deinit", "noinit", "users", "j", "J", "join", "l", "L", "leave", "n", "N", "name",
    "c", "c:", "chat", "raw", "html", "uhtml", "uhtmlchange", "notify", "tempnotify", "tempnotifyoff",
    "hidelines", "unlink", "userstats", "badge", "inactive", "inactiveoff", "t:", "timestamp", "sentchoice",
    "bigerror", "-hint", "debug", "player", "gametype", "gen", "tier", "rule", "rated", "seed", "teamsize",
    "request", "error",
})


@dataclass
class DecisionTiming:
    """Time rules of each decision in a room, in seconds from the receipt of the request (module docstring)."""
    search_s: float | None = None    # search allowance, given to the player as budget_ms; None: the bot's own setting
    deadline_s: float | None = None  # hard cap from receipt to sending the choice; None: no deadline
    min_response_s: float = 0.0      # computed decisions are not sent earlier than this (pacing)
    reserve_s: float = 0.5           # kept back from the deadline for the fallback and the send
    clock_margin_s: float = 10.0     # battle-clock seconds the bot never plans to use
    hang_s: float = 30.0             # a decision this long past its deadline marks the worker unhealthy

    @classmethod
    def from_dict(cls, values: dict | None) -> "DecisionTiming":
        known = {k: v for k, v in (values or {}).items() if k in cls.__dataclass_fields__}
        return cls(**known)


class RoomObserver(Protocol):
    def room_event(self, room: "BattleRoom", kind: str, data: dict) -> None: ...


# Ladder lines the room reads facts from (players' ratings, rated status, the battle timer, rating changes).
LADDER_LINES = frozenset({"player", "rated", "inactive", "inactiveoff", "raw", "", "-message"})
CLOCK_RE = re.compile(r"Time left: (\d+) sec this turn \| (\d+) sec total(?: \| (\d+) sec grace)?")
# Showdown's ladder: "NAME's rating: 1012 &rarr; <strong>1037</strong><br />(+25 for winning)" (NAME HTML-escaped).
RATING_RE = re.compile(r"(.+)'s rating: (\d+) &rarr; <strong>(\d+)</strong><br />\((.*)\)\s*$")
DELTA_RE = re.compile(r"([+-]?\d+) for ")


def to_id(text: str) -> str:
    return re.sub(r"[^a-z0-9]", "", text.lower())


def line_type(line: str) -> str:
    if not line.startswith("|"):
        return ""
    return line.split("|", 2)[1]


def decision_json(decision: Decision | None) -> dict | None:
    if decision is None:
        return None
    return {"choice": decision.choice, "a0": decision.a0, "a1": decision.a1, "analysis": decision.analysis}


DRIVER_KEYS = ("showdown",)  # config keys of the play server itself, not passed to the bot factory


def warm_up(bot: Bot) -> str:
    """Load what a bot would otherwise load on its first decision; returns what was loaded."""
    hook = getattr(bot, "warmup", None)
    if callable(hook):
        hook()
        return "warmup()"
    if getattr(bot, "search", None) is not None and isinstance(getattr(type(bot), "pool", None), property):
        bot.pool  # training.search.SearchBot: the team pool of the world builder (lazy, ~2 s)
        return "team pool"
    return ""


class BotWorker:
    """A bot, its config entry and the one thread that runs all of its players (and loads the bot)."""

    def __init__(self, spec: dict, bot: Bot | None = None):
        self.spec = spec
        self.bot = bot
        self.name: str = spec["name"]
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix=f"bot-{to_id(self.name)}")
        self.ready = asyncio.Event()
        self.failed: str | None = None
        if bot is not None:
            self.ready.set()

    async def run(self, fn, *args):
        return await asyncio.get_running_loop().run_in_executor(self.executor, functools.partial(fn, *args))

    async def load(self) -> None:
        """Make the bot (``training.player.load_bot``) and warm it up on its own thread, then set ``ready``."""
        from training.player import load_bot

        try:
            started = time.perf_counter()
            if self.bot is None:
                spec = {key: value for key, value in self.spec.items() if key not in DRIVER_KEYS}
                log.info("%s: loading %s", self.name, self.spec["factory"])
                self.bot = await self.run(load_bot, spec)
            loaded = time.perf_counter()
            what = await self.run(warm_up, self.bot)
            log.info("%s: ready (loaded in %.1f s%s)", self.name, loaded - started,
                     f", {what} in {time.perf_counter() - loaded:.1f} s" if what else "")
            self.ready.set()
        except Exception as error:
            self.failed = f"{type(error).__name__}: {error}"
            log.exception("%s: could not load the bot", self.name)

    def describe(self) -> dict:
        if self.bot is None:
            info = {"loading": self.failed is None, "error": self.failed}
        else:
            try:
                info = dict(self.bot.describe())
            except Exception as error:  # status must not fail because of one bot
                info = {"describe_error": repr(error)}
        info["name"] = self.name
        info["ready"] = self.ready.is_set()
        return info

    def shutdown(self) -> None:
        self.executor.shutdown(wait=False, cancel_futures=True)


PUBLIC_ANALYSIS = ("source", "win_probability", "value", "time_ms", "worlds", "cells")


def redact(entry: dict) -> dict:
    """A bot-log entry without what the opponent must not see while the battle runs: the choice, its action codes
    and any analysis built from the bot's own request (slots, candidates, opponent replies, notes)."""
    analysis = ((entry.get("decision") or {}).get("analysis")) or {}
    public = {key: analysis[key] for key in PUBLIC_ANALYSIS if key in analysis}
    return {**{key: entry.get(key) for key in ("bot", "side", "turn", "rqid", "time")},
            "decision": {"analysis": public}, "redacted": True}


class BotLog:
    """Decisions of the server's own bots, per room (``GET /api/bot-log``).

    A decision is added once the turn it belongs to is over (BattleRoom). While the battle runs, ``entries`` returns
    them redacted (``redact``) unless ``reveal``; after ``end(room)`` (``|win|``, ``|tie|``, room closed) in full."""

    def __init__(self, max_rooms: int = 200):
        self.rooms: OrderedDict[str, list[dict]] = OrderedDict()
        self.ended: set[str] = set()
        self.max_rooms = max_rooms

    def add(self, room: str, entry: dict) -> None:
        self.rooms.setdefault(room, []).append(entry)
        self.rooms.move_to_end(room)
        while len(self.rooms) > self.max_rooms:
            old, _ = self.rooms.popitem(last=False)
            self.ended.discard(old)

    def end(self, room: str) -> None:
        self.ended.add(room)

    def is_live(self, room: str) -> bool:
        return room not in self.ended

    def entries(self, room: str, reveal: bool = False) -> list[dict]:
        entries = list(self.rooms.get(room, []))
        if reveal or not self.is_live(room):
            return entries
        return [redact(entry) for entry in entries]


class BattleRoom:
    """One bot account's view of one battle room. Messages go into ``queue``; ``run`` processes them in order."""

    def __init__(self, room: str, worker: BotWorker, send: Callable[[str], Awaitable[None]], bot_log: BotLog,
                 log_path: Path | None, server: str, on_end: Callable[["BattleRoom"], None] | None = None, *,
                 observer: RoomObserver | None = None, timing: DecisionTiming | None = None,
                 rating_wait_s: float = 0.0, rejoined: bool = False):
        self.room = room
        self.worker = worker
        self.send_raw = send
        self.bot_log = bot_log
        self.log_path = log_path
        self.server = server
        self.on_end = on_end
        self.observer = observer
        self.timing = timing or DecisionTiming()
        self.rating_wait_s = rating_wait_s  # after a rated battle ends, wait this long for the rating lines
        self.rejoined = rejoined  # opened again after a reconnect: the player missed the battle so far
        self.userid = to_id(worker.name)
        self.queue: asyncio.Queue = asyncio.Queue()
        self.task: asyncio.Task | None = None
        self.player: Player | None = None

        self.unfed: list[str] = []
        self.fresh = False  # progress lines arrived since the last request was handed to the player
        self.pending_request: str | None = None  # received, not yet handed to the player
        self.pending_request_at: tuple[float, float] | None = None  # its receipt: (time.time(), time.monotonic())
        self.request_at: tuple[float, float] | None = None  # receipt of the request the player has now
        self.seq = 0  # decisions started in this room (requests, plus decisions again after a refusal)
        self.post_game_until: float | None = None  # after a rated battle: wait until then for the rating lines
        self.hold_until: float | None = None
        self.resend_until: float | None = None  # after [Unavailable choice]: waiting for the re-sent request
        self.redecide = False  # after [Invalid choice]: decide again on the current request
        self.request: dict | None = None  # the request the player has now
        self.awaiting: int | None = None  # rqid of the choice sent and not yet accepted or refused
        self.answered_rqid: int | None = None
        self.pending_entry: dict | None = None  # bot-log entry of the choice in flight (not accepted or refused yet)
        self.unpublished: list[dict] = []  # accepted choices of the turn in progress, published at its end
        self.refusals = 0  # refused choices on the current request

        self.turn = 0
        self.side: str | None = None
        self.ended = False
        self.closed = False
        self.result: dict | None = None
        self.stats = {"requests": 0, "decisions": 0, "refused": 0, "defaults": 0, "errors": 0, "held": 0,
                      "fallbacks": 0, "late": 0, "deadline_misses": 0, "unhealthy": 0}
        self.started = time.time()
        self.finished: float | None = None
        self.received: list[dict] = []
        self.sent: list[dict] = []
        self.decisions: list[dict] = []  # this bot's resolved bot-log entries in this room
        self.player_stats: dict | None = None  # the player's own ``stats`` dict at the end, if it has one
        # Ladder facts read off the room's lines.
        self.players: dict[str, dict] = {}  # "p1" -> {"name", "userid", "rating" (int or None)}
        self.rated: str | bool = False  # the |rated| line's text ("" counts as rated: True), False if none
        self.end_reason: str | None = None
        self.ratings: dict[str, dict] = {}  # userid -> the rating line's {old, new, delta, reason}
        self.timer_on: float | None = None  # when the server confirmed the battle timer
        self.clock: dict | None = None  # last "Time left" reading: {time, turn_left, total_left, grace}

    def _emit(self, kind: str, data: dict) -> None:
        if self.observer is None:
            return
        try:
            self.observer.room_event(self, kind, data)
        except Exception:  # recording must never break play
            log.exception("%s %s: observer failed on %s", self.worker.name, self.room, kind)

    # -- plumbing ----------------------------------------------------------------------------------------------

    def start(self) -> None:
        self.task = asyncio.create_task(self.run(), name=f"{self.worker.name}:{self.room}")

    def push(self, received_at: float, text: str) -> None:
        self.queue.put_nowait((received_at, text))

    def stop(self) -> None:
        """Abandon the room (connection lost, shutdown); ``run`` closes the player and writes the log."""
        self.queue.put_nowait(None)

    async def send(self, text: str) -> None:
        now = time.time()
        self.sent.append({"time": now, "text": text})
        self._emit("sent", {"time": now, "text": text})
        await self.send_raw(text)

    def _timeout(self) -> float | None:
        deadlines = [d for d in (self.hold_until, self.resend_until, self.post_game_until) if d is not None]
        return max(0.0, min(deadlines) - time.monotonic()) if deadlines else None

    def _awaiting_rating(self) -> bool:
        """After a rated battle: the server's rating lines for us have not come yet and the wait is not over."""
        return self.post_game_until is not None and self.userid not in self.ratings

    async def run(self) -> None:
        try:
            self.player = await self.worker.run(self.worker.bot.new_player, self.room)
            while not self.closed and not (self.ended and not self._awaiting_rating()):
                timeout = self._timeout()
                try:
                    item = await (asyncio.wait_for(self.queue.get(), timeout) if timeout is not None
                                  else self.queue.get())
                except TimeoutError:
                    await self._on_timeout()
                    continue
                if item is None:
                    break
                await self._on_message(*item)
        except asyncio.CancelledError:
            raise
        except Exception:
            self.stats["errors"] += 1
            log.exception("%s %s: room driver failed", self.worker.name, self.room)
        finally:
            await self._finish()

    # -- messages ----------------------------------------------------------------------------------------------

    async def _on_message(self, received_at: float, text: str) -> None:
        self.received.append({"time": received_at, "text": text})
        self._emit("message", {"time": received_at, "text": text})
        # The same instant on the monotonic clock, which the deadlines use.
        received_mono = time.monotonic() - max(0.0, time.time() - received_at)
        lines = text.split("\n")
        if lines and lines[0].startswith(">"):
            lines = lines[1:]
        if not self.received[:-1] and lines and lines[0] == "|init|battle":
            # The room opened mid-battle (a rejoin): the player sees the log but never saw our earlier requests.
            turns = [line for line in lines if line.startswith("|turn|")]
            if turns and turns[-1] != "|turn|1":
                self.rejoined = True
        errors = []
        for line in lines:
            kind = line_type(line)
            if kind == "request":
                body = line[len("|request|"):]
                if body and body != "null":
                    self.pending_request = body
                    self.pending_request_at = (received_at, received_mono)
                continue
            if kind == "error":
                errors.append(line[len("|error|"):])
                continue
            if not line:
                continue  # blank separator lines (the |init| message ends with two)
            self.unfed.append(line)
            if kind not in NON_PROGRESS:
                self.fresh = True
            if kind == "turn":
                try:
                    self.turn = int(line.split("|")[2])
                except (IndexError, ValueError):
                    pass
                self._publish()  # every choice sent so far was carried out
            elif kind == "win":
                self._on_end({"winner": line.split("|", 2)[2] if line.count("|") >= 2 else "", "tie": False})
            elif kind == "tie":
                self._on_end({"winner": None, "tie": True})
            elif kind in ("deinit", "noinit"):
                self.closed = True
            elif kind in LADDER_LINES:
                self._on_ladder_line(kind, line, received_at, received_mono)
        if self.ended or self.closed:
            return
        for message in errors:
            await self._on_error(message)
        if self.pending_request is not None:
            await self._maybe_dispatch()
        elif self.redecide:
            await self._redecide()

    def _on_ladder_line(self, kind: str, line: str, received_at: float, received_mono: float) -> None:
        """Players and their ratings, rated status, the battle timer and clock, rating changes, how it ended."""
        parts = line.split("|")
        body = line[len(kind) + 2:]
        if kind == "player":
            if len(parts) > 3 and parts[3]:
                rating = parts[5] if len(parts) > 5 else ""
                self.players[parts[2]] = {"name": parts[3], "userid": to_id(parts[3]),
                                          "rating": int(rating) if rating.isdigit() else None}
        elif kind == "rated":
            self.rated = body or True
        elif kind == "inactive":
            clock = CLOCK_RE.match(body)
            if clock:
                turn_left, total_left, grace = (int(g) if g else None for g in clock.groups())
                self.clock = {"time": received_at, "mono": received_mono, "turn_left": turn_left,
                              "total_left": total_left, "grace": grace}
                self._emit("clock", {"time": received_at, "turn_left": turn_left, "total_left": total_left,
                                     "grace": grace, "line": line})
            elif body.startswith("Battle timer is ON") or (
                    body.endswith("also wants the timer to be on.") and self.timer_on is None):
                if self.timer_on is None:
                    self.timer_on = received_at
                self._emit("timer", {"time": received_at, "state": "on", "line": line})
        elif kind == "inactiveoff":
            if body.startswith("Battle timer is now OFF"):
                self.timer_on = None
                self._emit("timer", {"time": received_at, "state": "off", "line": line})
        elif kind == "raw":
            rating = RATING_RE.match(body)
            if rating:
                name, old, new, reason = html.unescape(rating.group(1)), rating.group(2), rating.group(3), \
                    html.unescape(rating.group(4))
                delta = DELTA_RE.match(reason)
                entry = {"time": received_at, "user": name, "userid": to_id(name), "old": int(old), "new": int(new),
                         "delta": int(delta.group(1)) if delta else int(new) - int(old), "reason": reason,
                         "line": line}
                self.ratings[entry["userid"]] = entry
                self._emit("rating", entry)
            elif "Ratings not updated" in body or "rating changes" in body:
                self._emit("server_error", {"message": body})
        elif kind == "":
            if "Ladder isn't responding" in line:
                self._emit("server_error", {"message": line.strip("|")})
        elif kind == "-message":
            if body.endswith(" lost due to inactivity."):
                self.end_reason = "inactivity"
            elif "forfeit" in body:
                self.end_reason = "forfeit"

    def _on_end(self, result: dict) -> None:
        self.ended = True
        self.result = result
        winner = to_id(result.get("winner") or "")
        won = None if result.get("tie") or not winner else winner == self.userid
        if self.rated and self.rating_wait_s > 0 and self.userid not in self.ratings:
            self.post_game_until = time.monotonic() + self.rating_wait_s
        self._emit("end", {"time": time.time(), "result": result, "won": won,
                           "end_reason": self.end_reason or "normal", "turns": self.turn})

    async def _maybe_dispatch(self) -> None:
        try:
            request = json.loads(self.pending_request)
        except json.JSONDecodeError:
            log.warning("%s %s: unreadable request %.200s", self.worker.name, self.room, self.pending_request)
            self.pending_request = None
            return
        if not isinstance(request, dict):
            self.pending_request = None
            return
        if self.fresh or request.get("update") or self.resend_until is not None:
            await self._dispatch(request)
        elif self.hold_until is None:
            self.hold_until = time.monotonic() + HOLD_SECONDS

    async def _on_timeout(self) -> None:
        now = time.monotonic()
        if self.hold_until is not None and now >= self.hold_until:
            self.hold_until = None
            if self.pending_request is not None:
                self.stats["held"] += 1
                log.info("%s %s: request came without new battle lines; deciding anyway", self.worker.name, self.room)
                self.fresh = True
                await self._maybe_dispatch()
        if self.resend_until is not None and now >= self.resend_until:
            self.resend_until = None
            if self.pending_request is None:
                await self._redecide()
        if self.post_game_until is not None and now >= self.post_game_until:
            self.post_game_until = None
            if self.userid not in self.ratings:
                log.warning("%s %s: no rating line within %.0f s of the end", self.worker.name, self.room,
                            self.rating_wait_s)

    async def _dispatch(self, request: dict) -> None:
        text = self.pending_request
        received = self.pending_request_at or (time.time(), time.monotonic())
        self.pending_request = None
        self.pending_request_at = None
        self.hold_until = None
        lines, self.unfed, self.fresh = self.unfed, [], False
        resent = bool(request.get("update")) or self.resend_until is not None
        self.resend_until = None
        self.redecide = False
        rqid = request.get("rqid")
        if rqid is not None and rqid == self.answered_rqid and not resent:
            # The same request again (rejoining a room after reconnecting): our choice for it was already sent.
            if lines:
                await self._bounded(self.player.feed, lines)
            return
        if not resent:
            self._accept()
            self.refusals = 0
        self.request = request
        self.request_at = received
        self.stats["requests"] += 1
        self.seq += 1
        side = (request.get("side") or {}).get("id")
        if side:
            self.side = side
        self._emit("request", {"seq": self.seq, "rqid": rqid, "received": received[0], "text": text,
                               "lines": lines, "resent": resent, "clock": self._clock_reading(received[1])})
        if self.refusals >= 2:
            await self._bounded(self._feed_and_set, lines, text)
            if self.refusals >= 3:
                log.error("%s %s: even the default choice was refused; waiting for the server", self.worker.name,
                          self.room)
            else:
                await self._submit_default("the bot's choice was refused twice", self._meta(received))
            return
        await self._decide(lines, text, received)

    # -- deciding against the clock ----------------------------------------------------------------------------

    def _clock_reading(self, received_mono: float) -> dict | None:
        """The battle clock for the request received at ``received_mono``: the last "Time left" line, if it is not
        from an earlier request (the server sends one with every request while the timer is on)."""
        clock = self.clock
        if clock is None or clock["mono"] < received_mono - CLOCK_STALE_S:
            return None
        return {key: clock[key] for key in ("time", "turn_left", "total_left", "grace")}

    def _deadline(self, received_mono: float) -> float | None:
        """Monotonic time by which the choice must be sent: ``deadline_s`` after receipt less ``reserve_s``, and
        earlier when the battle clock has less left than that (keeping ``clock_margin_s``)."""
        timing = self.timing
        deadline = None if timing.deadline_s is None else received_mono + timing.deadline_s - timing.reserve_s
        clock = self.clock
        if clock is not None and clock["mono"] >= received_mono - CLOCK_STALE_S:
            left = [x for x in (clock["turn_left"], clock["total_left"]) if x is not None]
            if left:
                by_clock = clock["mono"] + min(left) - timing.clock_margin_s
                deadline = by_clock if deadline is None else min(deadline, by_clock)
        return deadline

    def _meta(self, received: tuple[float, float], deadline: float | None = None, budget_ms: float | None = None,
              **extra) -> dict:
        wall_deadline = None if deadline is None else time.time() + (deadline - time.monotonic())
        return {"seq": self.seq, "received": received[0], "dispatched": time.time(), "decided": None,
                "deadline": wall_deadline, "budget_ms": budget_ms, "queue_ms": None, "compute_ms": None,
                "paced_ms": 0.0, "fallback": None, **extra}

    async def _decide(self, lines: list[str] | None, text: str | None, received: tuple[float, float]) -> None:
        """Let the player decide on its worker thread under the room's time rules and send the result, or send a
        fallback at the deadline (module docstring, "Deadlines")."""
        timing = self.timing
        deadline = self._deadline(received[1])
        allowance_ms = None if timing.search_s is None else timing.search_s * 1000.0
        token = (self.room, self.seq)
        meta = self._meta(received, deadline, allowance_ms)
        future = asyncio.ensure_future(self.worker.run(self._think, lines, text, allowance_ms, deadline, token))
        if deadline is not None:
            await asyncio.wait({future}, timeout=max(0.0, deadline - time.monotonic()))
            if not future.done():
                await self._fallback(future, token, meta, "deadline")
                return
        try:
            decision, started, finished = await future
        except Exception:
            self.stats["errors"] += 1
            log.exception("%s %s: decide failed; sending default", self.worker.name, self.room)
            await self._submit_default("decide raised an exception", meta)
            return
        meta.update(decided=time.time(), queue_ms=round((started - received[1]) * 1000, 1),
                    compute_ms=round((finished - started) * 1000, 1))
        if decision is not None and timing.min_response_s > 0:
            target = received[1] + timing.min_response_s
            if deadline is not None:
                target = min(target, deadline)
            wait = target - time.monotonic()
            if wait > 0:
                meta["paced_ms"] = round(wait * 1000, 1)
                await asyncio.sleep(wait)
        await self._submit(decision, meta)

    def _think(self, lines: list[str] | None, request_text: str | None, allowance_ms: float | None,
               deadline: float | None, token) -> tuple[Decision | None, float, float]:
        """Worker thread: feed the lines and set the request (unless deciding again on the same one), hand the player
        its time (``budget_ms``, ``deadline``, ``decision_token``), decide. Returns (decision, start, end)."""
        started = time.monotonic()
        if request_text is not None:
            self._feed_and_set(lines, request_text)
        player = self.player
        if allowance_ms is not None or deadline is not None:
            budget = allowance_ms
            if deadline is not None:
                left = (deadline - time.monotonic()) * 1000.0 - THINK_MARGIN_MS
                budget = left if budget is None else min(budget, left)
            player.budget_ms = max(budget, 1.0)
            player.deadline = deadline
        player.decision_token = token
        decision = player.decide()
        return decision, started, time.monotonic()

    async def _fallback(self, future: asyncio.Future, token, meta: dict, reason: str) -> None:
        """The player missed the deadline: send the policy decision it posted early for this request, else
        ``default``. The player learns the choice actually sent once its late decision is done (``record_choice``
        queued behind it on the same worker thread, before any later request)."""
        self.stats["fallbacks"] += 1
        posted = getattr(self.player, "fallback", None)
        early = posted[1] if isinstance(posted, tuple) and len(posted) == 2 and posted[0] == token else None
        if early is not None:
            decision = Decision(early.choice, early.a0, early.a1, dict(early.analysis or {}))
            decision.analysis.setdefault("note", "early policy decision")
        else:
            decision = Decision("default", analysis={"source": "default", "note": "no policy decision yet"})
            self.stats["defaults"] += 1
        meta["fallback"] = reason if early is not None else f"{reason}, no policy decision yet"
        record = asyncio.ensure_future(self.worker.run(self.player.record_choice, decision.choice))
        record.add_done_callback(self._log_record_failure)
        seq = meta["seq"]
        rqid = (self.request or {}).get("rqid")
        future.add_done_callback(lambda done: self._on_late(done, seq, rqid))
        asyncio.get_running_loop().call_later(self.timing.hang_s, self._check_hang, future)
        log.warning("%s %s: no decision %.1f s after the request; sending %s %r", self.worker.name, self.room,
                    time.time() - meta["received"], "the early policy choice" if early else "default",
                    decision.choice)
        await self._send_choice(decision, meta, source="fallback")

    def _log_record_failure(self, future: asyncio.Future) -> None:
        if not future.cancelled() and future.exception() is not None:
            self.stats["errors"] += 1
            log.error("%s %s: record_choice after a fallback failed: %r", self.worker.name, self.room,
                      future.exception())

    def _on_late(self, future: asyncio.Future, seq: int, rqid) -> None:
        if future.cancelled():
            return
        self.stats["late"] += 1
        data = {"seq": seq, "rqid": rqid}
        if future.exception() is not None:
            data["error"] = repr(future.exception())
        else:
            decision, started, finished = future.result()
            data.update(choice=decision.choice if decision else None,
                        analysis=decision.analysis if decision else None,
                        compute_ms=round((finished - started) * 1000, 1))
        log.info("%s %s: late decision %s after %s ms", self.worker.name, self.room, data.get("choice"),
                 data.get("compute_ms"))
        self._emit("late_decision", data)

    def _check_hang(self, future: asyncio.Future) -> None:
        if future.done() or self.finished is not None:
            return
        self.stats["unhealthy"] += 1
        reason = f"a decision is still running {self.timing.hang_s:.0f} s after its deadline"
        log.error("%s %s: %s", self.worker.name, self.room, reason)
        self._emit("unhealthy", {"time": time.time(), "reason": reason})

    async def _bounded(self, fn, *args, timeout: float = 30.0):
        """``worker.run`` that gives up after ``timeout`` seconds (a stuck worker must not stall the room)."""
        future = asyncio.ensure_future(self.worker.run(fn, *args))
        try:
            return await asyncio.wait_for(asyncio.shield(future), timeout)
        except TimeoutError:
            self.stats["errors"] += 1
            log.error("%s %s: %s did not return within %.0f s", self.worker.name, self.room,
                      getattr(fn, "__name__", fn), timeout)
            return None

    def _feed_and_set(self, lines: list[str], request_text: str) -> None:
        if lines:
            self.player.feed(lines)
        self.player.set_request(request_text)

    async def _submit(self, decision: Decision | None, meta: dict) -> None:
        request = self.request or {}
        if request.get("wait"):
            return  # nothing to choose, whatever the player says
        if decision is None:
            if not request.get("teamPreview"):
                log.warning("%s %s: player returned no decision for a request that needs one; sending default",
                            self.worker.name, self.room)
            await self._submit_default("the player returned no decision", meta)
            return
        self.stats["decisions"] += 1
        await self._send_choice(decision, meta)

    async def _submit_default(self, note: str, meta: dict) -> None:
        """Let the server choose. The player did not make this choice, so it learns it like someone else's choice
        (``record_choice``): a SearchPlayer's world builder must replay every boundary."""
        self.stats["defaults"] += 1
        try:
            await self._bounded(self.player.record_choice, "default")
        except Exception:
            self.stats["errors"] += 1
            log.exception("%s %s: record_choice('default') failed", self.worker.name, self.room)
        await self._send_choice(Decision("default", analysis={"source": "default", "note": note}), meta)

    async def _send_choice(self, decision: Decision, meta: dict, source: str | None = None) -> None:
        rqid = (self.request or {}).get("rqid")
        self.awaiting = rqid
        self.answered_rqid = rqid
        self.pending_entry = {
            "bot": self.worker.name, "side": self.side, "turn": self.turn, "rqid": rqid, "time": time.time(),
            "decision": decision_json(decision),
        }
        suffix = f"|{rqid}" if rqid is not None else ""
        await self.send(f"{self.room}|/choose {decision.choice}{suffix}")
        submitted = time.time()
        total_ms = round((submitted - meta["received"]) * 1000, 1)
        if self.timing.deadline_s is not None and total_ms > self.timing.deadline_s * 1000:
            self.stats["deadline_misses"] += 1
            log.warning("%s %s: choice sent %.0f ms after its request (deadline %.0f ms)", self.worker.name,
                        self.room, total_ms, self.timing.deadline_s * 1000)
        analysis = decision.analysis or {}
        timing = {key: meta.get(key) for key in ("received", "dispatched", "decided", "deadline", "budget_ms",
                                                 "queue_ms", "compute_ms", "paced_ms")}
        timing.update(submitted=submitted, total_ms=total_ms)
        self._emit("decision", {"seq": meta["seq"], "rqid": rqid, "turn": self.turn, "choice": decision.choice,
                                "a0": decision.a0, "a1": decision.a1,
                                "source": source or analysis.get("source") or "unknown",
                                "fallback": meta.get("fallback"), "analysis": analysis, "timing": timing})

    def _accept(self) -> None:
        """A new request came: the choice in flight was accepted. It stays private until its turn is over: after a
        U-turn or Eject Button replacement request the partner's move of the same choice may still be queued."""
        if self.pending_entry is not None:
            self.unpublished.append(self.pending_entry)
        self.pending_entry = None
        self.awaiting = None

    def _publish(self) -> None:
        """The turn is over (``|turn|``, ``|win|``, ``|tie|``): its choices go to the bot log."""
        entries = self.unpublished + ([self.pending_entry] if self.pending_entry is not None else [])
        for entry in entries:
            self.bot_log.add(self.room, entry)
            self.decisions.append(entry)
        self.unpublished = []
        self.pending_entry = None

    async def _on_error(self, message: str) -> None:
        refused = message.startswith("[Invalid choice]") or message.startswith("[Unavailable choice]")
        stale = "too late" in message or "nothing to choose" in message
        if not refused or stale or self.awaiting is None:
            log.info("%s %s: server error: %s", self.worker.name, self.room, message)
            self._emit("server_error", {"message": message})
            return
        self.refusals += 1
        self.stats["refused"] += 1
        choice = (self.pending_entry or {}).get("decision", {}).get("choice")
        log.warning("%s %s: choice %r refused: %s", self.worker.name, self.room, choice, message)
        self._emit("refused", {"rqid": self.awaiting, "choice": choice, "message": message})
        self.pending_entry = None
        self.awaiting = None
        try:
            await self._bounded(self.player.rejected, message)
        except Exception:
            self.stats["errors"] += 1
            log.exception("%s %s: rejected() failed", self.worker.name, self.room)
        if message.startswith("[Unavailable choice]"):
            self.resend_until = time.monotonic() + HOLD_SECONDS
        else:
            self.redecide = True

    async def _redecide(self) -> None:
        """Decide again on the request the player already has (no re-sent request came). The server's clock kept
        running, so the decision gets a fresh window from now, still bounded by the battle clock."""
        self.redecide = False
        if self.request is None:
            return
        self.seq += 1
        received = (time.time(), time.monotonic())
        if self.refusals >= 3:
            log.error("%s %s: even the default choice was refused; waiting for the server", self.worker.name,
                      self.room)
            return
        if self.refusals >= 2:
            await self._submit_default("the bot's choice was refused twice", self._meta(received))
            return
        await self._decide(None, None, received)

    # -- end -----------------------------------------------------------------------------------------------------

    async def _finish(self) -> None:
        self.finished = time.time()
        if self.ended or self.closed:  # the battle is over (not just abandoned by a lost connection)
            self._publish()
            self.bot_log.end(self.room)
        if self.player is not None:
            lines, self.unfed = self.unfed, []
            try:
                await self._bounded(self._close_player, lines, timeout=10.0)
            except Exception:
                self.stats["errors"] += 1
                log.exception("%s %s: closing the player failed", self.worker.name, self.room)
        self.write_log()
        self._emit("closed", {"time": self.finished, "ended": self.ended, "abandoned": not (self.ended or self.closed),
                              "stats": dict(self.stats), "player_stats": self.player_stats, "rated": self.rated,
                              "players": self.players, "ratings": self.ratings})
        if self.on_end is not None:
            self.on_end(self)

    def _close_player(self, lines: list[str]) -> None:
        if lines:
            self.player.feed(lines)
        stats = getattr(self.player, "stats", None)
        if isinstance(stats, dict):
            self.player_stats = dict(stats)
        self.player.close()

    def write_log(self) -> None:
        if self.log_path is None:
            return
        data = {
            "room": self.room, "bot": self.worker.name, "userid": to_id(self.worker.name), "side": self.side,
            "server": self.server, "format": FORMAT, "started": self.started, "finished": self.finished,
            "result": self.result, "turns": self.turn, "stats": self.stats, "player_stats": self.player_stats,
            "decisions": self.decisions, "messages": self.received, "sent": self.sent,
        }
        try:
            self.log_path.parent.mkdir(parents=True, exist_ok=True)
            tmp = self.log_path.with_suffix(".tmp")
            tmp.write_text(json.dumps(data, ensure_ascii=False, indent=1, default=str))
            tmp.replace(self.log_path)
        except OSError:
            log.exception("%s %s: could not write %s", self.worker.name, self.room, self.log_path)
