"""Two-phase ``/api/advise`` (docs/play/ADVISOR-API.md, "Two-phase advice") with fake players.

    cd crates/pyengine && ../../.venv/bin/python -m pytest play/test_advise_phases.py -q

``PhasedPlayer`` behaves like a SearchPlayer with a second layer: ``decide`` publishes a layer-1 decision through
``on_layer1``, then "deepens" until the test releases it or ``cancel_depth2`` says stop.
"""
from __future__ import annotations

import asyncio
import json
import threading
import time
from types import SimpleNamespace

from aiohttp.test_utils import TestClient, TestServer

from play.api import Api
from play.driver import BotLog, BotWorker
from training.player import Bot, Decision, Player

LINES = [f"|line|{i}" for i in range(12)]


def request(rqid: int) -> str:
    return json.dumps({"side": {"id": "p1", "pokemon": []}, "rqid": rqid, "active": [{"moves": []}]})


def body(rqid: int, n_lines: int = 5, **extra) -> dict:
    history = [{"index": 2, "rqid": 1, "request": request(1), "choice": "move 1"}] if rqid > 1 else []
    history.append({"index": n_lines, "rqid": rqid, "request": request(rqid), "choice": None})
    return {"room": "battle-1", "lines": LINES[:n_lines], "request": request(rqid), "history": history, **extra}


class PhasedPlayer(Player):
    def __init__(self, bot: "PhasedBot"):
        super().__init__()
        self.bot = bot
        self.on_layer1 = None
        self.cancel_depth2 = None
        self.depth2_cancelled = False

    def set_request(self, request_json):
        super().set_request(request_json)
        self.rqid = json.loads(request_json)["rqid"]

    def decide(self):
        self.bot.decides.append(self.rqid)
        self.depth2_cancelled = False
        analysis = {"source": "search", "value": 0.2, "win_probability": 0.6,
                    "candidates": [{"choice": "move 1, move 1", "value": 0.2, "weight": 0.7}]}
        if self.on_layer1 is not None:
            self.on_layer1(Decision("move 1, move 1", 1, 1, dict(analysis)))
        # Layer 2: one "chunk" every 10 ms until released or cancelled.
        deadline = time.monotonic() + 10
        while not self.bot.release.is_set() and time.monotonic() < deadline:
            if self.bot.cancellable and self.cancel_depth2 is not None and self.cancel_depth2():
                self.depth2_cancelled = True
                break
            time.sleep(0.01)
        analysis["depth2"] = {"layer1_choice": "move 1, move 1", "chosen": "move 2, move 1", "changed": True,
                              "skipped": "cancelled" if self.depth2_cancelled else None,
                              "candidates": [{"choice": "move 1, move 1", "q": 0.2, "q2": [0.1]}]}
        analysis["time_ms"] = 123.0
        return Decision(f"move 2, move 1 [{self.rqid}]", 2, 1, analysis)


class PhasedBot(Bot):
    name = "Phased"

    def __init__(self):
        self.search = SimpleNamespace(depth2=SimpleNamespace(enabled=True))
        self.release = threading.Event()
        self.cancellable = True
        self.decides: list[int] = []

    def new_player(self, room):
        return PhasedPlayer(self)


class PlainPlayer(Player):
    """No two-phase hooks: a one-ply bot or a policy bot."""

    def decide(self):
        return Decision("move 3, move 4", analysis={"source": "search"})


class PlainBot(Bot):
    name = "Plain"

    def new_player(self, room):
        return PlainPlayer()


async def serve(bots: dict):
    workers = {name: BotWorker({"name": name}, bot) for name, bot in bots.items()}
    api = Api(workers, next(iter(bots)), {}, BotLog(), "ws://test")
    client = TestClient(TestServer(api.app()))
    await client.start_server()
    return api, client, workers


async def stop(api, client, workers):
    await client.close()
    await api.close()
    for worker in workers.values():
        worker.shutdown()


def run(coro):
    return asyncio.run(coro)


def test_status_reports_two_phase_support():
    async def go():
        api, client, workers = await serve({"Phased": PhasedBot(), "Plain": PlainBot()})
        try:
            data = await (await client.get("/api/status")).json()
            assert data["two_phase_advise"] is True
            assert {bot["name"]: bot["two_phase"] for bot in data["bots"]} == {"Phased": True, "Plain": False}
        finally:
            await stop(api, client, workers)

    run(go())


def test_phase_one_then_long_poll_for_phase_two():
    async def go():
        bot = PhasedBot()
        api, client, workers = await serve({"Phased": bot})
        try:
            response = await client.post("/api/advise", json=body(2, two_phase=True))
            first = await response.json()
            assert response.status == 200 and first["ok"]
            assert first["phase"] == 1 and first["pending"] is True and first["rqid"] == 2
            assert first["decision"]["choice"] == "move 1, move 1"
            assert "depth2" not in first["decision"]["analysis"] and "replay" not in first
            assert first["elapsed_ms"] < 2000
            advise_id = first["advise_id"]
            # Still deepening: a short poll says pending, without a decision.
            response = await client.get(f"/api/advise/{advise_id}", params={"timeout": "0.05"})
            waiting = await response.json()
            assert response.status == 200 and waiting["pending"] and waiting["phase"] == 1
            assert "decision" not in waiting
            poll = asyncio.ensure_future(client.get(f"/api/advise/{advise_id}", params={"timeout": "5"}))
            await asyncio.sleep(0.05)
            bot.release.set()
            response = await poll
            final = await response.json()
            assert response.status == 200 and final["phase"] == 2 and final["pending"] is False
            assert final["advise_id"] == advise_id and final["decision"]["choice"] == "move 2, move 1 [2]"
            assert final["decision"]["analysis"]["depth2"]["changed"] is True
            assert final["replay"]["replayed"] == 1 and final["elapsed_ms"] >= first["elapsed_ms"]
            # A finished result can be fetched again.
            again = await (await client.get(f"/api/advise/{advise_id}", params={"timeout": "0"})).json()
            assert again["decision"] == final["decision"]
        finally:
            await stop(api, client, workers)

    run(go())


def test_bot_without_second_layer_answers_once_and_old_clients_see_the_old_body():
    async def go():
        api, client, workers = await serve({"Plain": PlainBot()})
        try:
            data = await (await client.post("/api/advise", json=body(2, two_phase=True))).json()
            assert data["phase"] == 2 and data["pending"] is False and data["decision"]["choice"] == "move 3, move 4"
            assert set(data) == {"ok", "room", "rqid", "bot", "decision", "replay", "phase", "pending", "advise_id",
                                 "elapsed_ms"}
            # No two_phase: the original body, nothing added.
            old = await (await client.post("/api/advise", json=body(3, 8))).json()
            assert set(old) == {"ok", "room", "rqid", "bot", "decision", "replay"}
            assert old["rqid"] == 3
        finally:
            await stop(api, client, workers)

    run(go())


def test_single_phase_client_waits_for_the_two_ply_decision():
    async def go():
        bot = PhasedBot()
        api, client, workers = await serve({"Phased": bot})
        try:
            asyncio.get_running_loop().call_later(0.2, bot.release.set)
            response = await client.post("/api/advise", json=body(2))
            data = await response.json()
            assert response.status == 200 and set(data) == {"ok", "room", "rqid", "bot", "decision", "replay"}
            assert data["decision"]["choice"] == "move 2, move 1 [2]"
            assert data["decision"]["analysis"]["depth2"]["skipped"] is None
        finally:
            await stop(api, client, workers)

    run(go())


def test_a_newer_request_cancels_the_running_second_layer_and_its_result_is_never_served():
    async def go():
        bot = PhasedBot()
        api, client, workers = await serve({"Phased": bot})
        try:
            first = await (await client.post("/api/advise", json=body(2, two_phase=True))).json()
            assert first["pending"]
            poll = asyncio.ensure_future(client.get(f"/api/advise/{first['advise_id']}", params={"timeout": "10"}))
            await asyncio.sleep(0.05)
            started = time.monotonic()
            # The room moved on: the next request. The old second layer stops at its next chunk.
            response = await client.post("/api/advise", json=body(3, 8, two_phase=True))
            second = await response.json()
            assert time.monotonic() - started < 2, "the newer request did not wait for the old second layer"
            assert second["phase"] == 1 and second["rqid"] == 3
            stale = await poll
            assert stale.status == 409 and (await stale.json())["superseded"] is True
            stale = await client.get(f"/api/advise/{first['advise_id']}", params={"timeout": "0"})
            assert stale.status == 409
            bot.release.set()
            final = await (await client.get(f"/api/advise/{second['advise_id']}", params={"timeout": "5"})).json()
            assert final["phase"] == 2 and final["rqid"] == 3
            assert final["decision"]["analysis"]["depth2"]["skipped"] is None
        finally:
            await stop(api, client, workers)

    run(go())


def test_a_cancelled_decision_is_not_cached_for_the_same_request():
    async def go():
        bot = PhasedBot()
        api, client, workers = await serve({"Phased": bot})
        try:
            first = await (await client.post("/api/advise", json=body(2, two_phase=True))).json()
            # "Advise now" on the same request: supersedes, so the first one's layer 2 is cut short.
            second = await (await client.post("/api/advise", json=body(2, two_phase=True))).json()
            assert second["phase"] == 1 and second["advise_id"] != first["advise_id"]
            assert bot.decides == [2, 2], "the cut-short decision must not be served from the cache"
            bot.release.set()
            final = await (await client.get(f"/api/advise/{second['advise_id']}")).json()
            assert final["decision"]["analysis"]["depth2"]["skipped"] is None
            # Now the full decision is cached: asking again answers at once with phase 2.
            third = await (await client.post("/api/advise", json=body(2, two_phase=True))).json()
            assert third["phase"] == 2 and third["replay"] == {"cached": True} and bot.decides == [2, 2]
        finally:
            await stop(api, client, workers)

    run(go())


def test_a_request_waiting_for_the_room_is_superseded_with_409():
    async def go():
        bot = PhasedBot()
        api, client, workers = await serve({"Phased": bot})
        try:
            bot.cancellable = False  # the first decision runs on until released
            first = await (await client.post("/api/advise", json=body(2, two_phase=True))).json()
            assert first["pending"]
            # Phase 1 of the first is out; the second and third queue behind it. The third supersedes the second.
            second = asyncio.ensure_future(client.post("/api/advise", json=body(3, 8)))
            await asyncio.sleep(0.05)
            third = asyncio.ensure_future(client.post("/api/advise", json=body(4, 10, two_phase=True)))
            await asyncio.sleep(0.05)
            bot.cancellable = True
            response = await second
            assert response.status == 409 and not (await response.json())["ok"]
            data = await (await third).json()
            assert data["rqid"] == 4 and data["phase"] == 1
            bot.release.set()
        finally:
            await stop(api, client, workers)

    run(go())


def test_a_running_single_phase_request_is_never_cut_short():
    """Old clients: a newer call for the room waits for a running single-phase call, which runs to completion and
    answers exactly as before (no cancel hook, no extra keys)."""
    async def go():
        bot = PhasedBot()
        api, client, workers = await serve({"Phased": bot})
        try:
            first = asyncio.ensure_future(client.post("/api/advise", json=body(2)))
            await asyncio.sleep(0.1)
            second = asyncio.ensure_future(client.post("/api/advise", json=body(3, 8, two_phase=True)))
            await asyncio.sleep(0.3)
            assert not first.done() and not second.done(), "the single-phase search was cut short"
            bot.release.set()
            response = await first
            data = await response.json()
            assert response.status == 200 and set(data) == {"ok", "room", "rqid", "bot", "decision", "replay"}
            assert data["decision"]["analysis"]["depth2"]["skipped"] is None
            assert (await (await second).json())["rqid"] == 3
        finally:
            await stop(api, client, workers)

    run(go())


def test_a_superseded_queued_request_is_released_at_once():
    async def go():
        bot = PhasedBot()
        bot.cancellable = False  # the running decision holds the room until released
        api, client, workers = await serve({"Phased": bot})
        try:
            await client.post("/api/advise", json=body(2, two_phase=True))
            queued = asyncio.ensure_future(client.post("/api/advise", json=body(3, 8, two_phase=True)))
            await asyncio.sleep(0.05)
            newest = asyncio.ensure_future(client.post("/api/advise", json=body(4, 10, two_phase=True)))
            response = await asyncio.wait_for(queued, 1.0)  # long before the running decision ends
            assert response.status == 409 and (await response.json())["superseded"] is True
            assert not newest.done()
            active = [job for job in api.jobs.values() if not job.done.is_set()]
            assert len(active) == 2  # the running one and the newest
            bot.release.set()
            assert (await (await newest).json())["rqid"] == 4
        finally:
            await stop(api, client, workers)

    run(go())


def test_a_burst_beyond_the_active_cap_is_refused_and_the_registry_stays_bounded(monkeypatch):
    import play.api as api_module

    monkeypatch.setattr(api_module, "MAX_ACTIVE_JOBS", 3)
    monkeypatch.setattr(api_module, "MAX_JOBS", 5)

    async def go():
        bot = PhasedBot()
        bot.cancellable = False
        api, client, workers = await serve({"Phased": bot})
        try:
            calls = [asyncio.ensure_future(client.post("/api/advise", json=dict(body(2), room=f"battle-{i}",
                                                                                     two_phase=True)))
                     for i in range(3)]
            await asyncio.sleep(0.2)
            response = await client.post("/api/advise", json=dict(body(2), room="battle-9", two_phase=True))
            assert response.status == 429 and "too many" in (await response.json())["error"]
            response = await client.post("/api/advise", json=dict(body(2), room="battle-9"))  # single phase too
            assert response.status == 429
            assert len(api.jobs) == 3
            bot.release.set()
            ids = [(await (await call).json())["advise_id"] for call in calls]
            for advise_id in ids:
                await client.get(f"/api/advise/{advise_id}")
            # Many more finished calls: never more than MAX_JOBS kept, single-phase ones not at all.
            for i in range(8):
                await client.post("/api/advise", json=dict(body(2), room=f"battle-x{i}", two_phase=True))
                await client.post("/api/advise", json=dict(body(2), room=f"battle-y{i}"))
                assert len(api.jobs) <= 5
            assert all(job.phased for job in api.jobs.values())
        finally:
            await stop(api, client, workers)

    run(go())


def test_finished_results_expire_on_lookup_and_by_the_periodic_cleanup(monkeypatch):
    import play.api as api_module

    monkeypatch.setattr(api_module, "JOB_TTL_SECONDS", 0.2)
    monkeypatch.setattr(api_module, "PRUNE_INTERVAL_S", 0.05)

    async def go():
        api, client, workers = await serve({"Plain": PlainBot()})
        try:
            first = await (await client.post("/api/advise", json=body(2, two_phase=True))).json()
            assert (await client.get(f"/api/advise/{first['advise_id']}")).status == 200
            second = await (await client.post("/api/advise", json=dict(body(2), room="battle-2",
                                                                       two_phase=True))).json()
            await asyncio.sleep(0.4)
            assert second["advise_id"] not in api.jobs  # the periodic cleanup, without any request
            response = await client.get(f"/api/advise/{first['advise_id']}")
            assert response.status == 404
        finally:
            await stop(api, client, workers)

    run(go())


def test_an_expired_result_is_404_even_before_the_cleanup_runs(monkeypatch):
    import play.api as api_module

    monkeypatch.setattr(api_module, "JOB_TTL_SECONDS", 0.1)
    monkeypatch.setattr(api_module, "PRUNE_INTERVAL_S", 3600)

    async def go():
        api, client, workers = await serve({"Plain": PlainBot()})
        try:
            data = await (await client.post("/api/advise", json=body(2, two_phase=True))).json()
            await asyncio.sleep(0.2)
            assert data["advise_id"] in api.jobs
            assert (await client.get(f"/api/advise/{data['advise_id']}")).status == 404
            assert data["advise_id"] not in api.jobs
        finally:
            await stop(api, client, workers)

    run(go())


def test_poll_errors():
    async def go():
        api, client, workers = await serve({"Plain": PlainBot()})
        try:
            response = await client.get("/api/advise/nope")
            assert response.status == 404 and not (await response.json())["ok"]
            data = await (await client.post("/api/advise", json=body(2, two_phase=True))).json()
            response = await client.get(f"/api/advise/{data['advise_id']}", params={"timeout": "x"})
            assert response.status == 400
        finally:
            await stop(api, client, workers)

    run(go())
