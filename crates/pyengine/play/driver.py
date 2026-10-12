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
"""
from __future__ import annotations

import asyncio
import functools
import json
import logging
import re
import time
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Awaitable, Callable

from training.player import Bot, Decision, Player

log = logging.getLogger("play")

FORMAT = "gen9randomdoublesbattle"
HOLD_SECONDS = 1.0  # longest wait for the lines of a request (or for a re-sent request) before going without

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
                 log_path: Path | None, server: str, on_end: Callable[["BattleRoom"], None] | None = None):
        self.room = room
        self.worker = worker
        self.send_raw = send
        self.bot_log = bot_log
        self.log_path = log_path
        self.server = server
        self.on_end = on_end
        self.queue: asyncio.Queue = asyncio.Queue()
        self.task: asyncio.Task | None = None
        self.player: Player | None = None

        self.unfed: list[str] = []
        self.fresh = False  # progress lines arrived since the last request was handed to the player
        self.pending_request: str | None = None  # received, not yet handed to the player
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
        self.stats = {"requests": 0, "decisions": 0, "refused": 0, "defaults": 0, "errors": 0, "held": 0}
        self.started = time.time()
        self.finished: float | None = None
        self.received: list[dict] = []
        self.sent: list[dict] = []
        self.decisions: list[dict] = []  # this bot's resolved bot-log entries in this room
        self.player_stats: dict | None = None  # the player's own ``stats`` dict at the end, if it has one

    # -- plumbing ----------------------------------------------------------------------------------------------

    def start(self) -> None:
        self.task = asyncio.create_task(self.run(), name=f"{self.worker.name}:{self.room}")

    def push(self, received_at: float, text: str) -> None:
        self.queue.put_nowait((received_at, text))

    def stop(self) -> None:
        """Abandon the room (connection lost, shutdown); ``run`` closes the player and writes the log."""
        self.queue.put_nowait(None)

    async def send(self, text: str) -> None:
        self.sent.append({"time": time.time(), "text": text})
        await self.send_raw(text)

    def _timeout(self) -> float | None:
        deadlines = [d for d in (self.hold_until, self.resend_until) if d is not None]
        return max(0.0, min(deadlines) - time.monotonic()) if deadlines else None

    async def run(self) -> None:
        try:
            self.player = await self.worker.run(self.worker.bot.new_player, self.room)
            while not (self.ended or self.closed):
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
        lines = text.split("\n")
        if lines and lines[0].startswith(">"):
            lines = lines[1:]
        errors = []
        for line in lines:
            kind = line_type(line)
            if kind == "request":
                body = line[len("|request|"):]
                if body and body != "null":
                    self.pending_request = body
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
                self.ended = True
                self.result = {"winner": line.split("|", 2)[2] if line.count("|") >= 2 else "", "tie": False}
            elif kind == "tie":
                self.ended = True
                self.result = {"winner": None, "tie": True}
            elif kind in ("deinit", "noinit"):
                self.closed = True
        if self.ended or self.closed:
            return
        for message in errors:
            await self._on_error(message)
        if self.pending_request is not None:
            await self._maybe_dispatch()
        elif self.redecide:
            await self._redecide()

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

    async def _dispatch(self, request: dict) -> None:
        text = self.pending_request
        self.pending_request = None
        self.hold_until = None
        lines, self.unfed, self.fresh = self.unfed, [], False
        resent = bool(request.get("update")) or self.resend_until is not None
        self.resend_until = None
        self.redecide = False
        rqid = request.get("rqid")
        if rqid is not None and rqid == self.answered_rqid and not resent:
            # The same request again (rejoining a room after reconnecting): our choice for it was already sent.
            if lines:
                await self.worker.run(self.player.feed, lines)
            return
        if not resent:
            self._accept()
            self.refusals = 0
        self.request = request
        self.stats["requests"] += 1
        side = (request.get("side") or {}).get("id")
        if side:
            self.side = side
        if self.refusals >= 2:
            await self.worker.run(self._feed_and_set, lines, text)
            if self.refusals >= 3:
                log.error("%s %s: even the default choice was refused; waiting for the server", self.worker.name,
                          self.room)
            else:
                await self._submit_default("the bot's choice was refused twice")
            return
        try:
            decision = await self.worker.run(self._step, lines, text)
        except Exception:
            self.stats["errors"] += 1
            log.exception("%s %s: decide failed; sending default", self.worker.name, self.room)
            await self._submit_default("decide raised an exception")
            return
        await self._submit(decision)

    def _feed_and_set(self, lines: list[str], request_text: str) -> None:
        if lines:
            self.player.feed(lines)
        self.player.set_request(request_text)

    def _step(self, lines: list[str], request_text: str) -> Decision | None:
        self._feed_and_set(lines, request_text)
        return self.player.decide()

    async def _submit(self, decision: Decision | None) -> None:
        request = self.request or {}
        if request.get("wait"):
            return  # nothing to choose, whatever the player says
        if decision is None:
            if not request.get("teamPreview"):
                log.warning("%s %s: player returned no decision for a request that needs one; sending default",
                            self.worker.name, self.room)
            await self._submit_default("the player returned no decision")
            return
        self.stats["decisions"] += 1
        await self._send_choice(decision)

    async def _submit_default(self, note: str) -> None:
        """Let the server choose. The player did not make this choice, so it learns it like someone else's choice
        (``record_choice``): a SearchPlayer's world builder must replay every boundary."""
        self.stats["defaults"] += 1
        try:
            await self.worker.run(self.player.record_choice, "default")
        except Exception:
            self.stats["errors"] += 1
            log.exception("%s %s: record_choice('default') failed", self.worker.name, self.room)
        await self._send_choice(Decision("default", analysis={"source": "default", "note": note}))

    async def _send_choice(self, decision: Decision) -> None:
        rqid = (self.request or {}).get("rqid")
        self.awaiting = rqid
        self.answered_rqid = rqid
        self.pending_entry = {
            "bot": self.worker.name, "side": self.side, "turn": self.turn, "rqid": rqid, "time": time.time(),
            "decision": decision_json(decision),
        }
        suffix = f"|{rqid}" if rqid is not None else ""
        await self.send(f"{self.room}|/choose {decision.choice}{suffix}")

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
            return
        self.refusals += 1
        self.stats["refused"] += 1
        log.warning("%s %s: choice %r refused: %s", self.worker.name, self.room,
                    (self.pending_entry or {}).get("decision", {}).get("choice"), message)
        self.pending_entry = None
        self.awaiting = None
        try:
            await self.worker.run(self.player.rejected, message)
        except Exception:
            self.stats["errors"] += 1
            log.exception("%s %s: rejected() failed", self.worker.name, self.room)
        if message.startswith("[Unavailable choice]"):
            self.resend_until = time.monotonic() + HOLD_SECONDS
        else:
            self.redecide = True

    async def _redecide(self) -> None:
        """Decide again on the request the player already has (no re-sent request came)."""
        self.redecide = False
        if self.request is None:
            return
        if self.refusals >= 3:
            log.error("%s %s: even the default choice was refused; waiting for the server", self.worker.name,
                      self.room)
            return
        if self.refusals >= 2:
            await self._submit_default("the bot's choice was refused twice")
            return
        try:
            decision = await self.worker.run(self.player.decide)
        except Exception:
            self.stats["errors"] += 1
            log.exception("%s %s: decide failed; sending default", self.worker.name, self.room)
            await self._submit_default("decide raised an exception")
            return
        await self._submit(decision)

    # -- end -----------------------------------------------------------------------------------------------------

    async def _finish(self) -> None:
        self.finished = time.time()
        if self.ended or self.closed:  # the battle is over (not just abandoned by a lost connection)
            self._publish()
            self.bot_log.end(self.room)
        if self.player is not None:
            lines, self.unfed = self.unfed, []
            try:
                await self.worker.run(self._close_player, lines)
            except Exception:
                self.stats["errors"] += 1
                log.exception("%s %s: closing the player failed", self.worker.name, self.room)
        self.write_log()
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
