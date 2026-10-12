"""Protocol-driven players: one side of one battle, fed only what a Showdown client receives.

The same interface serves the in-process evaluation harness, the Showdown bot client and the Chrome extension's
advisor server, so a bot never reads a live engine battle (docs/design/SEARCH-PREREQUISITES.md item 1.4).

Event order. Players need the lines that lead to a request before the request. The pinned Showdown sends a
turn's lines first and the ``|request|`` right after (sim change "Emit requests after updates"); older servers
sent the request first, so a driver must make sure the update's lines were fed before ``set_request`` (see
play/driver.py). Within one battle the calls are therefore always::

    feed(lines) ... set_request(json) -> decide() -> feed(lines) ... set_request(json) -> decide() ...

``feed`` takes player-view protocol lines without the ``|request|`` lines (one line per item, no trailing newline).
Ladder-only lines (``|j|``, ``|c|``, ``|raw|``, timestamps, ...) may be passed; players ignore what they do not
understand. After a rejected choice (``|error|[Unavailable choice] ...`` or ``[Invalid choice]``) the driver calls
``rejected(message)``, then ``set_request`` with the re-sent request if there is one, and ``decide`` again.
"""
from __future__ import annotations

from dataclasses import dataclass, field


@dataclass
class Decision:
    """A choice for one request.

    ``choice`` is Showdown choice text without ``/choose`` or the rqid (``"move 1 +2 terastallize, switch 3"``).
    ``a0``/``a1`` are the action codes of docs/design/TRAINING-API.md ("Action space"), -1 when unknown.
    ``analysis`` is JSON-serialisable and is shown by the advisor (see docs/play/ADVISOR-API.md for its keys).
    """
    choice: str
    a0: int = -1
    a1: int = -1
    analysis: dict = field(default_factory=dict)


class Player:
    """One side of one battle. Subclasses implement ``decide``; the base class keeps the protocol state."""

    def __init__(self) -> None:
        self.lines: list[str] = []
        self.request_json: str | None = None
        self.side: int | None = None  # 0 = p1, 1 = p2, known from the first request

    def feed(self, lines: list[str]) -> None:
        self.lines.extend(lines)

    def set_request(self, request_json: str) -> None:
        self.request_json = request_json

    def rejected(self, message: str) -> None:
        """The last decision was refused by the server; a new ``set_request`` and ``decide`` follow."""

    def record_choice(self, choice: str) -> None:
        """Someone else answered the current request with ``choice`` (Showdown choice text): the advisor following
        a human's battle replays the human's past choices this way instead of calling ``decide``."""

    def decide(self) -> Decision | None:
        """The choice for the current request, or None when it needs none (``wait``, team preview)."""
        raise NotImplementedError

    def close(self) -> None:
        """The battle ended or was abandoned; release resources."""


class Bot:
    """Makes players that share one model. A driver calls a bot's ``decide`` methods from one thread at a time."""

    name = "bot"

    def new_player(self, room: str) -> Player:
        raise NotImplementedError

    def describe(self) -> dict:
        """JSON-serialisable summary for /api/status (checkpoint, search settings)."""
        return {"name": self.name}


def load_bot(spec: dict) -> Bot:
    """A bot from a config entry: ``{"name": ..., "factory": "module:callable", ...}``; the callable receives the
    whole entry as keyword arguments."""
    import importlib

    module, _, attribute = spec["factory"].partition(":")
    factory = getattr(importlib.import_module(module), attribute)
    return factory(**spec)


class DefaultPlayer(Player):
    """Lets the server choose (``default``: first legal move / first eligible switch). For harness tests."""

    def decide(self) -> Decision | None:
        import json

        request = json.loads(self.request_json) if self.request_json else {}
        if not request or request.get("wait") or request.get("teamPreview"):
            return None
        return Decision("default", analysis={"source": "default"})


class DefaultBot(Bot):
    def __init__(self, name: str = "DefaultBot", **_):
        self.name = name

    def new_player(self, room: str) -> Player:
        return DefaultPlayer()
