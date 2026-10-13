"""The play server's HTTP API (docs/play/ADVISOR-API.md): ``/api/status``, ``/api/advise``, ``/api/bot-log``.

Also ``POST /api/challenge`` (``{"bot": name, "user": name}``): the bot challenges the user to Random Doubles.

``/api/advise`` keeps one player per (bot, room) and feeds it the client's lines, or replays the client's request
history, on the bot's worker thread (play.advise). Calls for one room run one at a time; a newer call for the same
room supersedes an older one that is still waiting. A player that supports a per-call time budget exposes a
``budget_ms`` attribute, which is set from the request's ``budget_ms``. The response also has ``replay``: how
the call used the room (``cached``, ``rebuilt``, entries ``replayed``/``skipped``).

``/api/bot-log`` returns redacted entries while the room's battle runs (play.driver.redact: no choice, no analysis
built from the bot's own request), full ones after it ended, or always full with ``--reveal-bot-log``.
"""
from __future__ import annotations

import asyncio
import functools
import json
import logging
import time

from aiohttp import web

from play.advise import AdviseRoom, parse_history
from play.driver import BotLog, BotWorker, decision_json

log = logging.getLogger("play")

dumps = functools.partial(json.dumps, default=str)
CORS = {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type",
    "Access-Control-Allow-Private-Network": "true",
    "Access-Control-Max-Age": "600",
}
MAX_ADVISE_ROOMS = 32
ADVISE_ROOM_IDLE_SECONDS = 3 * 3600


def reply(data: dict, status: int = 200) -> web.Response:
    return web.json_response(data, status=status, dumps=dumps)


def fail(error: str, status: int = 400) -> web.Response:
    return reply({"ok": False, "error": error}, status)


class Api:
    def __init__(self, workers: dict[str, BotWorker], advisor: str | None, clients: dict, bot_log: BotLog,
                 server_url: str, reveal_bot_log: bool = False):
        self.workers = workers
        self.advisor = advisor
        self.clients = clients  # name -> ShowdownClient (bots logged in to Showdown)
        self.bot_log = bot_log
        self.server_url = server_url
        self.reveal_bot_log = reveal_bot_log  # full entries while battles run (bot-vs-bot watching)
        self.advise_rooms: dict[tuple[str, str], AdviseRoom] = {}

    def app(self) -> web.Application:
        app = web.Application(middlewares=[cors_middleware], client_max_size=64 * 1024 * 1024)
        app.router.add_get("/api/status", self.status)
        app.router.add_post("/api/advise", self.advise)
        app.router.add_get("/api/bot-log", self.bot_log_handler)
        app.router.add_post("/api/challenge", self.challenge)
        app.router.add_route("OPTIONS", "/{tail:.*}", options)
        return app

    # -- status ---------------------------------------------------------------------------------------------

    async def status(self, request: web.Request) -> web.Response:
        bots = []
        for name, worker in self.workers.items():
            info = worker.describe()
            client = self.clients.get(name)
            info["showdown"] = client is not None
            info["connected"] = bool(client and client.connected)
            info["battles"] = len(client.rooms) if client else 0
            info["advise_rooms"] = sum(1 for bot, _ in self.advise_rooms if bot == name)
            bots.append(info)
        return reply({"ok": True, "server": self.server_url, "advisor": self.advisor, "bots": bots})

    # -- advise ---------------------------------------------------------------------------------------------

    async def advise(self, request: web.Request) -> web.Response:
        try:
            body = await request.json()
        except (json.JSONDecodeError, UnicodeDecodeError):
            return fail("body is not JSON")
        if not isinstance(body, dict):
            return fail("body must be a JSON object")
        room = body.get("room")
        lines = body.get("lines")
        request_text = body.get("request")
        budget_ms = body.get("budget_ms")
        bot_name = body.get("bot") or self.advisor
        if not isinstance(room, str) or not room:
            return fail("room must be a non-empty string")
        if not isinstance(lines, list) or not all(isinstance(line, str) for line in lines):
            return fail("lines must be a list of strings")
        if isinstance(request_text, dict):
            request_text = json.dumps(request_text)
        if request_text is not None and not isinstance(request_text, str):
            return fail("request must be the request JSON text")
        if budget_ms is not None and (not isinstance(budget_ms, (int, float)) or isinstance(budget_ms, bool)):
            return fail("budget_ms must be a number")
        worker = self.workers.get(bot_name) if bot_name else None
        if worker is None:
            return fail(f"unknown bot {bot_name!r}; see /api/status" if bot_name else "no advisor bot configured",
                        404)
        if not worker.ready.is_set():
            return fail(f"bot {worker.name!r} is " + (f"unavailable: {worker.failed}" if worker.failed
                                                    else "still loading"), 503)
        if any(line.startswith("|request|") for line in lines):
            if "history" in body:  # indexes count the lines as sent
                return fail("lines must not contain |request| lines")
            lines = [line for line in lines if not line.startswith("|request|")]
        if request_text in ("", "null"):
            request_text = None
        history = None
        if body.get("history") is not None:
            try:
                history = parse_history(body["history"], len(lines), request_text)
            except ValueError as error:
                return fail(str(error))
            request_text = history[-1].request
        rqid = None
        if request_text is not None:
            try:
                parsed = json.loads(request_text)
            except json.JSONDecodeError:
                return fail("request is not valid JSON")
            if not isinstance(parsed, dict):
                return fail("request must be a JSON object")
            rqid = parsed.get("rqid")

        key = (worker.name, room)
        state = self.advise_rooms.get(key)
        if state is None:
            self._evict()
            state = self.advise_rooms[key] = AdviseRoom(worker.bot, room)
        state.seq += 1
        seq = state.seq
        async with state.lock:
            state.used = time.monotonic()
            if state.seq != seq:
                return reply({"ok": False, "error": "superseded by a newer request for this room", "room": room,
                              "rqid": rqid}, 409)
            try:
                if history is None:
                    decision, info = await worker.run(state.advise_lines, lines, request_text, budget_ms)
                else:
                    decision, info = await worker.run(state.advise_history, lines, history, budget_ms)
            except Exception as error:
                log.exception("advise %s %s failed", worker.name, room)
                await worker.run(state.close)
                return fail(f"{type(error).__name__}: {error}", 500)
            analysis = getattr(decision, "analysis", None) or {}
            if getattr(worker.bot, "search", None) and analysis.get("source") != "search":
                log.info("advise %s %s: no search (%s)", worker.name, room, analysis.get("note") or "no note")
            return reply({"ok": True, "room": room, "rqid": rqid, "bot": worker.name,
                          "decision": decision_json(decision), "replay": info})

    def _evict(self) -> None:
        now = time.monotonic()
        stale = [key for key, state in self.advise_rooms.items()
                 if now - state.used > ADVISE_ROOM_IDLE_SECONDS and not state.lock.locked()]
        while len(self.advise_rooms) - len(stale) >= MAX_ADVISE_ROOMS:
            idle = sorted((state.used, key) for key, state in self.advise_rooms.items()
                          if key not in stale and not state.lock.locked())
            if not idle:
                break
            stale.append(idle[0][1])
        for key in stale:
            state = self.advise_rooms.pop(key)
            asyncio.create_task(self.workers[key[0]].run(state.close))

    async def close(self) -> None:
        for (name, _), state in list(self.advise_rooms.items()):
            try:
                await self.workers[name].run(state.close)
            except Exception:
                log.exception("closing advise player failed")
        self.advise_rooms.clear()


    # -- bot log, challenge -------------------------------------------------------------------------------------

    async def bot_log_handler(self, request: web.Request) -> web.Response:
        room = request.query.get("room")
        if not room:
            rooms = [{"room": name, "entries": len(entries)} for name, entries in self.bot_log.rooms.items()]
            return reply({"ok": True, "rooms": rooms})
        redacted = self.bot_log.is_live(room) and not self.reveal_bot_log
        return reply({"ok": True, "room": room, "live": self.bot_log.is_live(room), "redacted": redacted,
                      "entries": self.bot_log.entries(room, reveal=self.reveal_bot_log)})

    async def challenge(self, request: web.Request) -> web.Response:
        try:
            body = await request.json()
        except (json.JSONDecodeError, UnicodeDecodeError):
            return fail("body is not JSON")
        user = body.get("user") if isinstance(body, dict) else None
        name = (body.get("bot") if isinstance(body, dict) else None) or next(iter(self.clients), None)
        client = self.clients.get(name)
        if not isinstance(user, str) or not user:
            return fail("user must be a Showdown user name")
        if client is None or not client.connected:
            return fail(f"bot {name!r} is not logged in to Showdown", 404)
        await client.challenge(user)
        return reply({"ok": True, "bot": name, "user": user})


async def options(request: web.Request) -> web.Response:
    return web.Response(status=204)


@web.middleware
async def cors_middleware(request: web.Request, handler):
    try:
        response = await handler(request)
    except web.HTTPException as error:
        response = reply({"ok": False, "error": error.reason}, error.status)
    except Exception as error:
        log.exception("API handler failed")
        response = reply({"ok": False, "error": f"{type(error).__name__}: {error}"}, 500)
    response.headers.update(CORS)
    return response
