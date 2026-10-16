"""The play server's HTTP API (docs/play/ADVISOR-API.md): ``/api/status``, ``/api/advise``, ``/api/bot-log``.

Also ``POST /api/challenge`` (``{"bot": name, "user": name}``): the bot challenges the user to Random Doubles.

``/api/advise`` keeps one player per (bot, room) and feeds it the client's lines, or replays the client's request
history, on the bot's worker thread (play.advise). Calls for one room run one at a time; a newer call for the same
room supersedes an older one that is still waiting. A player that supports a per-call time budget exposes a
``budget_ms`` attribute, which is set from the request's ``budget_ms``. The response also has ``replay``: how
the call used the room (``cached``, ``rebuilt``, entries ``replayed``/``skipped``).

Two-phase advice (``"two_phase": true``, docs/play/ADVISOR-API.md): every advise call runs as an ``AdviseJob``
task that holds the room's lock while the worker decides. The player's layer-1 decision comes from the worker
thread through ``PhaseHooks.on_layer1`` (``loop.call_soon_threadsafe``); the POST answers with it (phase 1) or with
the final decision, whichever exists first, and ``GET /api/advise/<id>`` long-polls the final one. A newer call for
the same (bot, room) supersedes the room's earlier jobs: queued ones are answered 409 at once; a running two-phase
job's ``PhaseHooks.cancel`` stops its second layer before its next chunk, and its result is never served. A running
single-phase job gets no hooks and runs to completion, as before. At most ``MAX_ACTIVE_JOBS`` calls run or wait
(429 beyond); finished two-phase results are kept ``JOB_TTL_SECONDS`` (pruned on completion, on new calls, every
``PRUNE_INTERVAL_S`` and on lookup), at most ``MAX_JOBS`` jobs in all.

``/api/bot-log`` returns redacted entries while the room's battle runs (play.driver.redact: no choice, no analysis
built from the bot's own request), full ones after it ended, or always full with ``--reveal-bot-log``.
"""
from __future__ import annotations

import asyncio
import functools
import json
import logging
import secrets
import time

from aiohttp import web

from play.advise import AdviseRoom, PhaseHooks, parse_history
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
JOB_TTL_SECONDS = 300  # a finished two-phase result stays fetchable this long
MAX_JOBS = 256  # jobs kept (running, queued and finished two-phase ones); the oldest finished go first
MAX_ACTIVE_JOBS = 64  # running or queued advise calls; more are refused with 429
PRUNE_INTERVAL_S = 30.0
POLL_DEFAULT_S, POLL_MAX_S = 20.0, 60.0
SUPERSEDED = "superseded by a newer request for this room"


def reply(data: dict, status: int = 200) -> web.Response:
    return web.json_response(data, status=status, dumps=dumps)


def fail(error: str, status: int = 400) -> web.Response:
    return reply({"ok": False, "error": error}, status)


class AdviseJob:
    """One ``POST /api/advise`` call: its phase-1 (layer-1) decision and its final result, set on the event loop."""

    def __init__(self, room: str, bot: str, rqid, seq: int, received: float, phased: bool = False,
                 on_finish=None):
        self.id = secrets.token_hex(8)
        self.room, self.bot, self.rqid, self.seq, self.received = room, bot, rqid, seq, received
        self.phased = phased  # a two-phase call: its second layer may be cancelled, its result fetched by id
        self.on_finish = on_finish
        self.started = False  # holds the room and is deciding (not queued)
        self.hooks = PhaseHooks()
        self.phase1: dict | None = None  # decision_json of the layer-1 decision
        self.phase1_ms: float | None = None
        self.result: tuple[int, dict] | None = None  # (HTTP status, body) of the final answer
        self.final_ms: float | None = None
        self.finished_at: float | None = None
        self.superseded = False
        self.published = asyncio.Event()  # phase 1 or the final result exists
        self.done = asyncio.Event()
        self.wake = asyncio.Event()  # done or superseded: long-polls return
        self.task: asyncio.Task | None = None

    def ms_since_received(self, at: float) -> float:
        return round((at - self.received) * 1000, 1)

    def supersede(self) -> None:
        """A newer call for the same room came. Queued: answered 409 now (the task stops waiting for the room).
        Running: a two-phase call's second layer stops at its next chunk; a single-phase call runs to the end, as
        before two-phase advice existed."""
        self.superseded = True
        if self.phased:
            self.hooks.cancel.set()
        if not self.started and not self.done.is_set():
            self.finish(409, {"ok": False, "error": SUPERSEDED, "room": self.room, "rqid": self.rqid})
            if self.task is not None:
                self.task.cancel()
        self.wake.set()

    def publish_phase1(self, decision: dict, at: float) -> None:
        if self.phase1 is None and not self.done.is_set():
            self.phase1, self.phase1_ms = decision, self.ms_since_received(at)
            self.published.set()

    def finish(self, status: int, body: dict) -> None:
        if self.done.is_set():
            return
        self.finished_at = time.monotonic()
        self.result, self.final_ms = (status, body), self.ms_since_received(self.finished_at)
        self.done.set()
        self.published.set()
        self.wake.set()
        if self.on_finish is not None:
            self.on_finish(self)

    def expired(self, now: float) -> bool:
        return self.finished_at is not None and now - self.finished_at > JOB_TTL_SECONDS

    def head(self) -> dict:
        return {"room": self.room, "rqid": self.rqid, "bot": self.bot, "advise_id": self.id}

    def superseded_reply(self) -> web.Response:
        return reply({"ok": False, "superseded": True, "error": SUPERSEDED, **self.head()}, 409)

    def final_reply(self) -> web.Response:
        """The final result for a two-phase client (phase 2), or 409 when a newer request superseded it."""
        if self.superseded:
            return self.superseded_reply()
        status, body = self.result
        if status == 200:
            body = dict(body, phase=2, pending=False, advise_id=self.id, elapsed_ms=self.final_ms)
        else:
            body = dict(body, advise_id=self.id)
        return reply(body, status)


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
        self.jobs: dict[str, AdviseJob] = {}

    def app(self) -> web.Application:
        app = web.Application(middlewares=[cors_middleware], client_max_size=64 * 1024 * 1024)
        app.router.add_get("/api/status", self.status)
        app.router.add_post("/api/advise", self.advise)
        app.router.add_get("/api/advise/{id}", self.advise_poll)
        app.router.add_get("/api/bot-log", self.bot_log_handler)
        app.router.add_post("/api/challenge", self.challenge)
        app.router.add_route("OPTIONS", "/{tail:.*}", options)
        app.cleanup_ctx.append(self._janitor)
        return app

    async def _janitor(self, app: web.Application):
        """Prune expired advise jobs every PRUNE_INTERVAL_S while the app runs."""
        async def loop():
            while True:
                await asyncio.sleep(PRUNE_INTERVAL_S)
                self._prune_jobs()

        task = asyncio.create_task(loop())
        yield
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)

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
            info["two_phase"] = two_phase(worker.bot)
            bots.append(info)
        return reply({"ok": True, "server": self.server_url, "advisor": self.advisor, "two_phase_advise": True,
                      "bots": bots})

    # -- advise ---------------------------------------------------------------------------------------------

    async def advise(self, request: web.Request) -> web.Response:
        received = time.monotonic()
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
        phased = body.get("two_phase") is True
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
        # Admission: queued calls of this room are about to be released; everything else that is not done counts.
        active = sum(1 for old in self.jobs.values()
                     if not old.done.is_set() and not ((old.bot, old.room) == key and not old.started))
        if active >= MAX_ACTIVE_JOBS:
            return fail(f"too many advise requests in progress ({active}); try again later", 429)
        state.seq += 1
        for old in list(self.jobs.values()):
            if (old.bot, old.room) == key and not old.superseded:
                old.supersede()
        self._prune_jobs(room_for=1)
        job = AdviseJob(room, worker.name, rqid, state.seq, received, phased, self._job_finished)
        self.jobs[job.id] = job
        job.task = asyncio.create_task(self._run_job(worker, state, job, lines, request_text, history, budget_ms,
                                                     phased))
        if not phased:
            await job.done.wait()
            status, data = job.result
            return reply(data, status)
        await job.published.wait()
        if job.done.is_set():
            return job.final_reply()
        if job.superseded:
            return job.superseded_reply()
        return reply({"ok": True, **job.head(), "phase": 1, "pending": True, "elapsed_ms": job.phase1_ms,
                      "decision": job.phase1})

    async def _run_job(self, worker: BotWorker, state: AdviseRoom, job: AdviseJob, lines, request_text, history,
                       budget_ms, phased: bool) -> None:
        """Decide for one advise call on the bot's worker, holding the room's lock until the decision is done."""
        try:
            async with state.lock:
                state.used = time.monotonic()
                if state.seq != job.seq or job.done.is_set():
                    job.finish(409, {"ok": False, "error": SUPERSEDED, "room": job.room, "rqid": job.rqid})
                    return
                job.started = True
                hooks = None  # single phase: no hooks, the search always runs to completion (as before)
                if phased:
                    hooks = job.hooks
                    loop = asyncio.get_running_loop()

                    def on_layer1(decision) -> None:  # the worker thread, mid-decide
                        at = time.monotonic()
                        copy = json.loads(dumps(decision_json(decision)))  # the decision keeps changing after this
                        loop.call_soon_threadsafe(job.publish_phase1, copy, at)

                    hooks.on_layer1 = on_layer1
                try:
                    if history is None:
                        decision, info = await worker.run(state.advise_lines, lines, request_text, budget_ms, hooks)
                    else:
                        decision, info = await worker.run(state.advise_history, lines, history, budget_ms, hooks)
                except Exception as error:
                    log.exception("advise %s %s failed", worker.name, job.room)
                    await worker.run(state.close)
                    job.finish(500, {"ok": False, "error": f"{type(error).__name__}: {error}"})
                    return
                analysis = getattr(decision, "analysis", None) or {}
                if getattr(worker.bot, "search", None) and analysis.get("source") != "search":
                    log.info("advise %s %s: no search (%s)", worker.name, job.room, analysis.get("note") or "no note")
                job.finish(200, {"ok": True, "room": job.room, "rqid": job.rqid, "bot": worker.name,
                                 "decision": decision_json(decision), "replay": info})
        except asyncio.CancelledError:
            job.finish(503, {"ok": False, "error": "the server is shutting down"})
            raise
        except Exception as error:  # never leave a client waiting
            log.exception("advise job %s %s failed", worker.name, job.room)
            job.finish(500, {"ok": False, "error": f"{type(error).__name__}: {error}"})

    async def advise_poll(self, request: web.Request) -> web.Response:
        """``GET /api/advise/<id>?timeout=S``: the final decision of a two-phase request (long-poll)."""
        job = self.jobs.get(request.match_info["id"])
        if job is not None and job.expired(time.monotonic()):
            del self.jobs[job.id]
            job = None
        if job is None or not job.phased:
            return fail("unknown or expired advise id", 404)
        try:
            timeout = float(request.query.get("timeout", POLL_DEFAULT_S))
        except ValueError:
            return fail("timeout must be a number of seconds")
        timeout = min(max(timeout, 0.0), POLL_MAX_S) if timeout == timeout else POLL_DEFAULT_S
        if not job.wake.is_set():
            try:
                await asyncio.wait_for(job.wake.wait(), timeout)
            except TimeoutError:
                pass
        if job.superseded:
            return job.superseded_reply()
        if not job.done.is_set():
            return reply({"ok": True, **job.head(), "phase": 1, "pending": True})
        return job.final_reply()

    def _job_finished(self, job: AdviseJob) -> None:
        if not job.phased:  # nobody can fetch a single-phase result by id
            self.jobs.pop(job.id, None)
        self._prune_jobs()

    def _prune_jobs(self, room_for: int = 0) -> None:
        """Drop finished jobs older than JOB_TTL_SECONDS, and the oldest finished ones beyond MAX_JOBS (keeping
        ``room_for`` free places). Running and queued jobs stay (MAX_ACTIVE_JOBS bounds them)."""
        now = time.monotonic()
        finished = sorted((job.finished_at, job.id) for job in self.jobs.values() if job.finished_at is not None)
        excess = len(self.jobs) + room_for - MAX_JOBS
        for at, job_id in finished:
            if now - at > JOB_TTL_SECONDS or excess > 0:
                del self.jobs[job_id]
                excess -= 1

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
        for job in list(self.jobs.values()):
            job.supersede()  # queued calls are answered, running second layers stop at their next chunk
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


def two_phase(bot) -> bool:
    """The bot's decisions have a second search layer (two-phase advice)."""
    depth2 = getattr(getattr(bot, "search", None), "depth2", None)
    return bool(depth2 is not None and getattr(depth2, "enabled", False))


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
