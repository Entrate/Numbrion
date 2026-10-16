"""Advisor rooms for ``POST /api/advise`` (docs/play/ADVISOR-API.md): one player per (bot, room).

Without ``history`` the client's ``lines`` are fed as they come: only the suffix after the lines already fed, or a
new player when that prefix differs; then ``set_request`` and ``decide``.

With ``history`` (every request of the room with the user's choice for it, the last one current) the room replays
it: for each earlier entry, feed ``lines[:index]`` not yet fed, ``set_request``, then ``record_choice`` if the
entry has a choice; for the last entry, feed up to its index, ``set_request`` and ``decide``. Entries already
replayed (same index, rqid and choice) are skipped. A changed replayed entry, or lines that no longer match what
was fed, rebuild the player.

The advisor's own ``decide`` is only a suggestion. If it returned one, the room calls ``rejected`` before the next
request is replayed or the same one is decided again, so the player drops it: a player that remembers its last
decision as our choice (SearchPlayer's ``pending_choice``) must not take the suggestion for what the user chose.
The user's real choice then comes with ``set_request`` + ``record_choice``, as after a refused choice. When
``decide`` returned None (a ``wait`` request) there is nothing to drop, and ``rejected`` would also drop the user's
previous choice, which such a player still holds until its next update.

Two-phase advice (``PhaseHooks``): a player that supports it (SearchPlayer with a second layer) gets
``on_layer1`` and ``cancel_depth2`` for the one ``decide``; ``on_layer1`` runs on the worker thread with the
layer-1 decision. A decision whose second layer was cancelled (the player's ``depth2_cancelled``) is not cached.

The methods run on the bot's worker thread, one call per room at a time (the API holds the room's lock).
"""
from __future__ import annotations

import asyncio
import json
import threading
import time
from dataclasses import dataclass, field
from typing import Callable

from training.player import Bot, Decision

USER_CHOSE = "advice superseded: replaying the user's own choice for this request"
ASKED_AGAIN = "advice superseded: the same request is being advised again"
UNCONFIRMED = "advice superseded: the user's choice for this request is unknown"


@dataclass(frozen=True)
class Entry:
    index: int
    rqid: int | None
    request: str | None
    choice: str | None

    def key(self) -> tuple:
        return (self.index, self.rqid, self.choice)

    def boundary(self) -> tuple:
        return (self.index, self.rqid, self.request)


def parse_history(raw, n_lines: int, request_text: str | None) -> list[Entry]:
    """Validated history entries; raises ValueError with a message for the client."""
    if not isinstance(raw, list) or not raw:
        raise ValueError("history must be a non-empty list")
    entries, previous = [], 0
    for i, item in enumerate(raw):
        if not isinstance(item, dict):
            raise ValueError(f"history[{i}] must be an object")
        index, rqid, request, choice = item.get("index"), item.get("rqid"), item.get("request"), item.get("choice")
        if isinstance(request, dict):
            request = json.dumps(request)
        if not isinstance(index, int) or isinstance(index, bool) or not 0 <= index <= n_lines:
            raise ValueError(f"history[{i}].index must be an integer in [0, len(lines)]")
        if index < previous:
            raise ValueError(f"history[{i}].index is smaller than the previous entry's")
        if rqid is not None and (not isinstance(rqid, int) or isinstance(rqid, bool)):
            raise ValueError(f"history[{i}].rqid must be an integer or null")
        if request is not None and not isinstance(request, str):
            raise ValueError(f"history[{i}].request must be the request JSON text")
        if choice is not None and not isinstance(choice, str):
            raise ValueError(f"history[{i}].choice must be a string or null")
        if request in ("", "null"):
            request = None
        entries.append(Entry(index, rqid, request, choice or None))
        previous = index
    if request_text is not None and entries[-1].request != request_text:
        raise ValueError("the last history entry must be the current request")
    return entries


@dataclass
class PhaseHooks:
    """One advise call's link to a two-phase player: the layer-1 listener and the cancel flag (thread-safe)."""
    on_layer1: Callable[[Decision], None] | None = None
    cancel: threading.Event = field(default_factory=threading.Event)

    def cancelled(self) -> bool:
        return self.cancel.is_set()


@dataclass
class AdviseRoom:
    bot: Bot
    room: str
    player: object = None
    mode: str | None = None  # "lines" or "history"
    fed: list[str] = field(default_factory=list)
    replayed: list[tuple] = field(default_factory=list)  # keys of the earlier entries replayed
    current: tuple | None = None  # the boundary the player last decided on
    suggested: bool = False  # that decide returned a decision (a suggestion the player may hold as ours)
    cached: tuple | None = None  # (key, decision) of the last answer
    lock: asyncio.Lock = field(default_factory=asyncio.Lock)
    seq: int = 0
    used: float = field(default_factory=time.monotonic)

    # -- shared --------------------------------------------------------------------------------------------------

    def close(self) -> None:
        player, self.player = self.player, None
        self.mode, self.fed, self.replayed, self.current, self.cached = None, [], [], None, None
        self.suggested = False
        if player is not None:
            player.close()

    def _new_player(self, mode: str) -> None:
        self.close()
        self.player = self.bot.new_player(self.room)
        self.mode = mode

    def _feed_to(self, lines: list[str], index: int) -> int:
        n = len(self.fed)
        if index <= n:
            return 0
        self.player.feed(lines[n:index])
        self.fed = list(lines[:index])
        return index - n

    def _decide(self, request: str, budget_ms, hooks: PhaseHooks | None = None) -> Decision | None:
        if budget_ms is not None and hasattr(self.player, "budget_ms"):
            self.player.budget_ms = budget_ms
        self.player.set_request(request)
        player = self.player
        hooked = hooks is not None and hasattr(player, "on_layer1")
        if hooked:
            player.on_layer1, player.cancel_depth2 = hooks.on_layer1, hooks.cancel.is_set
        try:
            decision = player.decide()
        finally:
            if hooked:
                player.on_layer1 = player.cancel_depth2 = None
        self.suggested = decision is not None
        return decision

    def _cacheable(self, key, decision):
        """The answer to cache: none when the player's second layer was cancelled (cut short)."""
        return None if getattr(self.player, "depth2_cancelled", False) else (key, decision)

    def _drop_suggestion(self, reason: str) -> None:
        if self.suggested:
            self.player.rejected(reason)
        self.suggested = False
        self.current = None

    # -- without history ------------------------------------------------------------------------------------------

    def advise_lines(self, lines: list[str], request: str | None, budget_ms=None,
                     hooks: PhaseHooks | None = None) -> tuple[Decision | None, dict]:
        n = len(self.fed)
        rebuild = self.player is None or self.mode != "lines" or len(lines) < n or lines[:n] != self.fed
        key = ("lines", request, budget_ms)
        if not rebuild and len(lines) == n and self.cached is not None and self.cached[0] == key:
            return self.cached[1], {"cached": True}
        if rebuild:
            self._new_player("lines")
        else:
            self._drop_suggestion(UNCONFIRMED)
        fed = self._feed_to(lines, len(lines))
        decision = None
        if request is not None:
            decision = self._decide(request, budget_ms, hooks)
            self.current = ("lines",)
        self.cached = self._cacheable(key, decision)
        return decision, {"rebuilt": rebuild, "fed": fed}

    # -- with history ---------------------------------------------------------------------------------------------

    def _rebuild_reason(self, lines: list[str], earlier: list[Entry], last: Entry) -> str:
        if self.player is None or self.mode != "history":
            return "new room"
        n = len(self.fed)
        if len(lines) < n or lines[:n] != self.fed:
            return "lines differ from the ones already fed"
        r = len(self.replayed)
        if len(earlier) < r or [entry.key() for entry in earlier[:r]] != self.replayed:
            return "a replayed history entry changed"
        position = n
        for i, entry in enumerate(earlier[r:] + [last]):
            if i == 0 and self.current is not None:
                if entry.boundary() != self.current:
                    return "the advised request is not the next history entry"
                continue
            if entry.index < position:
                return "history entry before lines already fed"
            position = entry.index
        return ""

    def advise_history(self, lines: list[str], history: list[Entry], budget_ms=None,
                       hooks: PhaseHooks | None = None) -> tuple[Decision | None, dict]:
        earlier, last = history[:-1], history[-1]
        reason = self._rebuild_reason(lines, earlier, last)
        key = ("history", last.boundary(), budget_ms)
        if (not reason and len(self.replayed) == len(earlier) and self.current == last.boundary()
                and self.cached is not None and self.cached[0] == key):
            return self.cached[1], {"cached": True}
        if reason:
            self._new_player("history")
        player = self.player
        replayed = 0
        for entry in earlier[len(self.replayed):]:
            if self.current is not None and entry.boundary() == self.current:
                self._drop_suggestion(USER_CHOSE)  # its lines and request were given when we advised on it
            else:
                self._feed_to(lines, entry.index)
            if entry.request is not None:
                player.set_request(entry.request)
                if entry.choice is not None:
                    player.record_choice(entry.choice)
            self.replayed.append(entry.key())
            replayed += 1
        if self.current is not None:  # the same request again (more lines arrived, another budget)
            self._drop_suggestion(ASKED_AGAIN)
        self._feed_to(lines, last.index)
        decision = None
        if last.request is not None:
            decision = self._decide(last.request, budget_ms, hooks)
        self.current = last.boundary()
        self.cached = self._cacheable(key, decision)
        return decision, {"rebuilt": bool(reason), "reason": reason or None, "replayed": replayed,
                          "skipped": len(earlier) - replayed}
