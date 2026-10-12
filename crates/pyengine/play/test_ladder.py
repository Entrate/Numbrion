"""Ladder recording and lifecycle tests: instant bots, temporary files, and 127.0.0.1 servers only."""
from __future__ import annotations

import asyncio
import gzip
import json
import sqlite3
import time
from pathlib import Path
from types import SimpleNamespace

import aiohttp
import pytest
import websockets
from aiohttp import web

from play import ladder
from play import supervise
from play.auth import AuthError, Credentials, load_credentials, login_assertion
from play.driver import BotWorker
from play.ladder import LadderClient, load_config
from play.ladder_report import inspect_decision, main as report_main, report
from play.runstore import RunStore, read_battle
from play.supervise import child_args
from training.player import DefaultBot

ROOM = "battle-gen9randomdoublesbattle-1"
PASSWORD = "do-not-record-this-password-492"
ASSERTION = "do-not-record-this-assertion-539"


def config(**updates):
    return load_config(None, base={"account": "NumbrionLadder", "bot": {"factory": "training.player:DefaultBot"},
                                   "server": "ws://127.0.0.1:1/showdown/websocket", "users_json": None,
                                   "timing": {"search_s": .01, "deadline_s": .5, "min_response_s": 0},
                                   "rating_wait_s": .15, "stop": {"minutes": None, "games": 1}, **updates})


def manifest(cfg=None, run_id="test-run"):
    cfg = cfg or config()
    return {"run_id": run_id, "account": cfg["account"], "server": cfg["server"], "format": cfg["format"],
            "started": time.time(), "config": cfg, "config_sha256": ladder.json_hash(cfg), "timing": cfg["timing"],
            "bot": {"spec": cfg["bot"], "checkpoint": None}, "code": {"commit": "test"}}


def make_store(tmp_path, cfg=None, run_id="test-run"):
    m = manifest(cfg, run_id)
    m["runs_dir"] = str(tmp_path / "runs")
    return RunStore(tmp_path / "runs", tmp_path / "ladder.sqlite", m, (PASSWORD, ASSERTION))


def fake_room():
    return SimpleNamespace(room=ROOM, side="p1", turn=1, rated=True, rejoined=False, timer_on=None,
                           started=time.time(), players={"p1": {"name": "NumbrionLadder", "rating": 1012},
                                                         "p2": {"name": "Opp", "rating": 1234}})


def request_body(rqid=1):
    return {"rqid": rqid, "active": [{"moves": [{"move": "Tackle", "id": "tackle", "target": "normal"}]}] * 2,
            "side": {"name": "NumbrionLadder", "id": "p1", "pokemon":
                     [{"ident": "p1: Pikachu", "details": "Pikachu, L80", "condition": "100/100"},
                      {"ident": "p1: Eevee", "details": "Eevee, L80", "condition": "100/100"}]}}


def decision(seq=1, probability=.8, source="search", **extra):
    return {"seq": seq, "rqid": seq, "turn": seq, "choice": "move 1 1 terastallize, move 1 2",
            "a0": 1, "a1": 2, "source": source, "fallback": None,
            "analysis": {"source": source, "win_probability": probability, "value": .6, "worlds": 4, "cells": 20,
                         "policy": {"choice": "move 2 1, move 1 2", "text": "policy text", "prior": .7},
                         "candidates": [{"choice": "move 1 1 terastallize, move 1 2", "text": "Tackle",
                                         "prior": .3, "value": .6, "weight": .8},
                                        {"choice": "move 2 1, move 1 2", "text": "policy text", "value": .59}],
                         "opponent": [{"text": "Opp: Protect", "prior": .5, "weight": .6}],
                         "observed": {"species": "Charizard"}, "inferred": {"item": "Leftovers"}},
            "timing": {"received": time.time() - .6, "submitted": time.time(), "total_ms": 600, "compute_ms": 300,
                       "queue_ms": 50, "paced_ms": 250, "budget_ms": 10, "deadline": time.monotonic()}, **extra}


def connect_db(tmp_path):
    conn = sqlite3.connect(tmp_path / "ladder.sqlite")
    conn.row_factory = sqlite3.Row
    return conn


async def http_server(handler):
    app = web.Application()
    app.router.add_post("/api/login", handler)
    app.router.add_post("/action.php", handler)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    port = site._server.sockets[0].getsockname()[1]
    return runner, f"http://127.0.0.1:{port}/api/login"


@pytest.mark.parametrize("first", [404, 503, "bad JSON"])
def test_login_legacy_fallback(first):
    async def scenario():
        forms = []
        async def handle(request):
            forms.append(dict(await request.post()))
            if request.path == "/api/login":
                return web.Response(status=first) if isinstance(first, int) else web.Response(text=first)
            return web.Response(text=']{"actionsuccess":true,"assertion":"abc"}')
        runner, url = await http_server(handle)
        try:
            async with aiohttp.ClientSession() as session:
                assert await login_assertion(session, "A", PASSWORD, "challenge", url) == "abc"
            assert forms == [{"name": "A", "pass": PASSWORD, "challstr": "challenge"},
                             {"act": "login", "name": "A", "pass": PASSWORD, "challstr": "challenge"}]
        finally:
            await runner.cleanup()
    asyncio.run(scenario())


def test_auth_errors_and_credentials(tmp_path, monkeypatch):
    monkeypatch.delenv("NUMBRION_PS_USERNAME", raising=False)
    monkeypatch.delenv("NUMBRION_PS_PASSWORD", raising=False)
    path = tmp_path / "credentials.json"
    path.write_text(json.dumps({"username": "A", "password": PASSWORD}))
    assert load_credentials(path).password == PASSWORD
    assert PASSWORD not in repr(load_credentials(path))
    monkeypatch.setenv("NUMBRION_PS_USERNAME", "B")
    with pytest.raises(AuthError):
        load_credentials(path)
    monkeypatch.setenv("NUMBRION_PS_PASSWORD", "env-password")
    assert load_credentials(path).username == "B"
    async def scenario():
        async def handle(request):
            return web.Response(text="]" + json.dumps({"actionsuccess": False,
                                                      "assertion": ";;Wrong password " + PASSWORD}))
        runner, url = await http_server(handle)
        try:
            async with aiohttp.ClientSession() as session:
                with pytest.raises(AuthError, match="Wrong password") as raised:
                    await login_assertion(session, "A", PASSWORD, "challenge", url)
                assert PASSWORD not in str(raised.value)
        finally:
            await runner.cleanup()
    asyncio.run(scenario())


def test_recorder_gzip_sql_and_report(tmp_path, capsys):
    store = make_store(tmp_path)
    room = fake_room()
    try:
        store.emit("connect", {})
        store.emit("message", {"text": "|challstr|secret-challenge"})
        store.emit("sent", {"text": "|/trn A,0," + ASSERTION})
        store.emit("popup", {"text": PASSWORD})
        store.emit("battle_open", {"queue_wait_s": 1.25, "rejoined": False}, ROOM)
        store.emit("timer_sent", {}, ROOM)
        store.room_event(room, "timer", {"time": time.time(), "state": "on", "line": "timer is ON"})
        store.room_event(room, "message", {"time": time.time(), "text": ">" + ROOM + "\n|turn|1"})
        store.room_event(room, "request", {"seq": 1, "rqid": 1, "received": time.time(),
                                           "text": json.dumps(request_body()), "lines": [], "resent": False})
        store.room_event(room, "message", {"text": "|switch|p2a: Z|Charizard, L80|100/100"})
        store.room_event(room, "decision", decision())
        room.turn = 2
        store.room_event(room, "request", {"seq": 2, "rqid": 2, "text": json.dumps(request_body(2))})
        store.room_event(room, "decision", decision(2, .4, "fallback", fallback="deadline"))
        store.room_event(room, "refused", {"rqid": 2, "choice": "default", "message": "invalid choice"})
        store.room_event(room, "late_decision", {"seq": 2, "rqid": 2, "choice": "move 1", "compute_ms": 800})
        store.room_event(room, "rating", {"userid": "numbrionladder", "user": "NumbrionLadder", "old": 1012,
                                          "new": 1037, "delta": 25, "reason": "winning"})
        store.room_event(room, "end", {"won": True, "result": {"winner": "NumbrionLadder", "tie": False},
                                       "end_reason": "normal", "turns": 2})
        store.room_event(room, "closed", {"ended": True, "abandoned": False, "stats": {"errors": 1}, "rated": True,
                                          "players": room.players})
        store.emit("ratings_snapshot", {"rating": {"elo": 1037, "gxe": 60, "rpr": 1100, "rprd": 50, "w": 1, "l": 0}})
        store.emit("resources", {"cpu": 80, "proc_cpu": 2, "rss_mb": 123, "mem_percent": 70, "worker_queue": 0,
                                 "rooms": 0})
        store.finish("complete").result(timeout=5)
        assert not store.failed.is_set()
    finally:
        store.close()
    gz = tmp_path / "runs" / "test-run" / "battles" / f"{ROOM}.jsonl.gz"
    assert gz.exists() and not gz.with_suffix("").exists()
    assert any(e["kind"] == "decision" for e in read_battle(gz))
    with connect_db(tmp_path) as conn:
        battle = dict(conn.execute("SELECT * FROM battles").fetchone())
        assert battle["result"] == "win" and battle["rating_delta_room"] == 25
        assert battle["decisions"] == 2 and battle["fallback"] == 1 and battle["search"] == 1
        assert battle["opponent"] == "Opp" and battle["opp_rating_before"] == 1234
        assert json.loads(battle["our_species"]) == ["Pikachu", "Eevee"]
        assert json.loads(battle["opp_species"]) == ["Charizard"]
        assert battle["tera_used"] and battle["timer_on"] and battle["timer_sent"]
        output = report(conn, "test-run")
        assert "1/0/0" in output and "1037" in output and "Brier score" in output
        assert f"{ROOM}:1" in output and "probability dropped 0.40" in output
        assert "value gain only 0.0100" in output
        assert "600.0" in output and "Technical problem" in output
        full = inspect_decision(conn, f"{ROOM}:1")
        assert "Pikachu" in full and "Opp: Protect" in full and "Leftovers" in full and "Tackle" in full
        assert "|turn|1" in full and '"total_ms": 600' in full
    assert report_main(["--db", str(tmp_path / "ladder.sqlite"), "--run", "test-run"]) == 0
    assert (tmp_path / "runs" / "test-run" / "report.md").exists()
    assert report_main(["--db", str(tmp_path / "ladder.sqlite"), "--run", "test-run", "--decision", f"{ROOM}:1"]) == 0
    assert "Full decision event" in capsys.readouterr().out
    for path in tmp_path.rglob("*"):
        if path.is_file():
            payload = gzip.open(path, "rb").read() if path.suffix == ".gz" else path.read_bytes()
            assert PASSWORD.encode() not in payload and ASSERTION.encode() not in payload
            assert b"secret-challenge" not in payload


def test_pending_resume_segments_and_unknown(tmp_path):
    store = make_store(tmp_path)
    room = fake_room()
    store.emit("battle_open", {"rejoined": False}, ROOM)
    store.room_event(room, "decision", decision())
    store.room_event(room, "closed", {"ended": False, "abandoned": True, "stats": {}, "rated": True})
    store.flush().result(timeout=5)
    with connect_db(tmp_path) as conn:
        assert conn.execute("SELECT result FROM battles").fetchone()[0] is None
    store.close()
    store = make_store(tmp_path)
    assert store.initial_battles[ROOM]["result"] is None
    store.emit("battle_open", {"rejoined": True}, ROOM)
    store.room_event(room, "decision", decision())
    store.finish("complete").result(timeout=5)
    store.close()
    with connect_db(tmp_path) as conn:
        b = conn.execute("SELECT * FROM battles").fetchone()
        assert b["result"] == "unknown" and b["segments"] == 2 and b["rejoins"] == 1
        assert [r[0] for r in conn.execute("SELECT seq FROM decisions ORDER BY seq")] == [1, 2]


def test_recorder_write_failure(tmp_path, monkeypatch):
    store = make_store(tmp_path)
    def broken(*args):
        raise OSError(PASSWORD)
    monkeypatch.setattr(store, "_project", broken)
    store.emit("popup", {"text": "test"})
    store.flush().result(timeout=5)
    assert store.failed.is_set() and PASSWORD not in store.failure
    store.close()


def test_resume_refuses_checkpoint_change_and_config(tmp_path, monkeypatch):
    checkpoint = tmp_path / "tiny.pt"
    checkpoint.write_bytes(b"first")
    cfg = config(bot={"factory": "training.player:DefaultBot", "checkpoint": str(checkpoint)})
    monkeypatch.setattr(ladder, "code_identity", lambda: {"commit": "test", "branch": "ladder-runner", "dirty": True})
    monkeypatch.setattr(ladder, "extension_identity", lambda: {"module": None, "native": [], "SEARCH": False})
    old = ladder.build_manifest(cfg, "run")
    checkpoint.write_bytes(b"changed")
    new = ladder.build_manifest(cfg, "run")
    with pytest.raises(ladder.ConfigError, match="checkpoint"):
        ladder.validate_resume(old, new)
    assert ladder.validate_resume(old, new, force=True) == ["checkpoint"]
    old["config_sha256"] = "different"
    with pytest.raises(ladder.ConfigError, match="config"):
        ladder.validate_resume(old, new)


def test_limits_health_and_search_gates(tmp_path):
    async def scenario():
        cfg = config(local=True, stop={"minutes": None, "games": 1})
        store = make_store(tmp_path, cfg)
        worker = BotWorker(cfg["bot"], DefaultBot(cfg["account"]))
        client = LadderClient(worker, cfg, store)
        sends = []
        async def send(text):
            sends.append(text)
        client.send = send
        client.logged_in.set()
        client.reconcile_at = 0
        try:
            client.games.add(ROOM)
            await client.maybe_search()
            assert not sends
            client.games.clear()
            client.rooms[ROOM] = fake_room()
            await client.maybe_search()
            assert not sends
            client.rooms.clear()
            client.after_tasks.add(object())
            await client.maybe_search()
            assert not sends
            client.after_tasks.clear()
            await client.maybe_search()
            assert sends == ["|/utm null", "|/search gen9randomdoublesbattle"]
            client.finished_count = 1
            await client.maybe_search()
            assert client.stop_requested and len(sends) == 2
            client.request_stop(force=True)
            assert client.force_stop
            client.stop_requested = False
            for i in range(3):
                await client.auth_failed("Wrong password " + PASSWORD)
                assert client.next_auth_at >= time.monotonic() + (5, 30, 120)[i] - .1
            assert client.exit_code == 3
        finally:
            worker.shutdown()
            store.close()
    asyncio.run(scenario())


def test_resume_original_start_and_supervisor_arguments(tmp_path):
    cfg = config(stop={"minutes": 1, "games": None})
    store = make_store(tmp_path, cfg)
    worker = BotWorker(cfg["bot"], DefaultBot(cfg["account"]))
    try:
        store.manifest["started"] = time.time() - 61
        client = LadderClient(worker, cfg, store)
        client.check_limits()
        assert client.stop_requested
    finally:
        worker.shutdown()
        store.close()
    assert child_args(["--config", "x", "--run-id", "old"], "new", True) == ["--config", "x", "--resume", "new"]
    assert child_args(["--resume=old", "--games", "1"], "new", False) == ["--games", "1", "--run-id", "new"]


async def fake_ladder_run(tmp_path, finish=False, reconnect=False, games=1):
    """A registered-login handshake and complete ladder script, with no external sockets or model loading."""
    async def login(request):
        form = await request.post()
        assert form["pass"] == PASSWORD
        return web.Response(text="]" + json.dumps({"actionsuccess": True, "assertion": ASSERTION}))
    http, login_url = await http_server(login)
    cfg = config(login_url=login_url, stop={"minutes": None, "games": games})
    store = make_store(tmp_path, cfg)
    worker = BotWorker(cfg["bot"], DefaultBot(cfg["account"]))
    sent, connections, errors = [], 0, []
    scripted = asyncio.Event()
    client = None
    async def receive(ws):
        text = await asyncio.wait_for(ws.recv(), 4)
        sent.append(text)
        return text
    async def handler(ws):
        nonlocal connections
        connections += 1
        segment = connections
        try:
            await ws.send("|challstr|1|very-private-challenge-string")
            assert await receive(ws) == f"|/trn {cfg['account']},0,{ASSERTION}"
            await ws.send(f"|updateuser| {cfg['account']}|1|1|{{}}")
            if segment == 1:
                await ws.send('|updatesearch|{"searching":[],"games":null}')
                assert await receive(ws) == "|/utm null"
                assert await receive(ws) == "|/search gen9randomdoublesbattle"
                await ws.send('|updatesearch|{"searching":["gen9randomdoublesbattle"],"games":null}')
            else:
                await ws.send("|updatesearch|" + json.dumps({"searching": [], "games": {ROOM: "A vs B"}}))
                assert await receive(ws) == f"|/join {ROOM}"
                assert not any("/search " in text for text in sent[3:])
            await ws.send("|updatesearch|" + json.dumps({"searching": [], "games": {ROOM: "A vs B"}}))
            await ws.send(f">{ROOM}\n|init|battle\n|title|NumbrionLadder vs Opp\n|j|NumbrionLadder\n"
                          "|player|p1|NumbrionLadder|1|1012\n|player|p2|Opp|2|1234\n|rated|\n|start\n"
                          f"|turn|{segment}\n|request|" + json.dumps(request_body(segment)))
            assert await receive(ws) == f"{ROOM}|/timer on"
            assert await receive(ws) == f"{ROOM}|/choose default|{segment}"
            await ws.send("|pm| Stranger| NumbrionLadder|/challenge gen9randomdoublesbattle|||\n"
                          "|pm| Stranger| NumbrionLadder|hello")
            assert await receive(ws) == "|/reject stranger"
            assert not any("/pm " in text or "/accept " in text for text in sent)
            if reconnect and segment == 1:
                await asyncio.wrap_future(store.flush())
                with connect_db(tmp_path) as conn:
                    assert conn.execute("SELECT result FROM battles").fetchone()[0] is None
                await ws.close()
                return
            if finish:
                await ws.send(f">{ROOM}\n|win|NumbrionLadder")
                await asyncio.sleep(.02)
                await ws.send(f">{ROOM}\n|raw|NumbrionLadder's rating: 1012 &rarr; <strong>1037</strong>"
                              "<br />(+25 for winning)\n|raw|Opp's rating: 1234 &rarr; <strong>1209</strong>"
                              "<br />(-25 for losing)")
                assert await receive(ws) == f"{ROOM}|/leave"
                await ws.send('|updatesearch|{"searching":[],"games":null}')
                if games == 2:
                    assert await receive(ws) == "|/utm null"
                    assert await receive(ws) == "|/search gen9randomdoublesbattle"
                    client.request_stop("test finished")
                    assert await receive(ws) == "|/cancelsearch"
            scripted.set()
            await ws.wait_closed()
        except Exception as error:
            errors.append(error)
            scripted.set()
            await ws.close()
    try:
        async with aiohttp.ClientSession() as session:
            async with websockets.serve(handler, "127.0.0.1", 0) as server:
                port = server.sockets[0].getsockname()[1]
                cfg["server"] = f"ws://127.0.0.1:{port}/showdown/websocket"
                client = LadderClient(worker, cfg, store, Credentials(cfg["account"], PASSWORD), session)
                original_update = client._on_updateuser
                def updated(line):
                    original_update(line)
                    client.reconcile_at = time.monotonic()
                client._on_updateuser = updated
                running = asyncio.create_task(client.drive())
                try:
                    await asyncio.wait_for(scripted.wait(), 8)
                    if errors:
                        raise errors[0]
                    if not finish:
                        client.request_stop(force=True)
                    code = await asyncio.wait_for(running, 4)
                    assert code == (0 if finish else 130)
                finally:
                    if not running.done():
                        running.cancel()
                    await asyncio.gather(running, return_exceptions=True)
        await asyncio.wrap_future(store.finish("complete"))
    finally:
        worker.shutdown()
        store.close()
        await http.cleanup()
    assert not errors
    for path in tmp_path.rglob("*"):
        if path.is_file():
            payload = gzip.open(path, "rb").read() if path.suffix == ".gz" else path.read_bytes()
            assert PASSWORD.encode() not in payload and ASSERTION.encode() not in payload
            assert b"very-private-challenge-string" not in payload
    return sent


def test_localhost_registered_login_matchmaking_timer_and_challenge(tmp_path):
    sent = asyncio.run(fake_ladder_run(tmp_path))
    assert sent.index(f"{ROOM}|/timer on") < sent.index(f"{ROOM}|/choose default|1")
    with connect_db(tmp_path) as conn:
        assert conn.execute("SELECT result FROM battles").fetchone()[0] == "unknown"


def test_localhost_reconnect_rejoins_without_loss_or_new_search(tmp_path):
    sent = asyncio.run(fake_ladder_run(tmp_path, reconnect=True))
    assert sum(text == "|/search gen9randomdoublesbattle" for text in sent) == 1
    assert f"|/join {ROOM}" in sent
    with connect_db(tmp_path) as conn:
        b = conn.execute("SELECT * FROM battles").fetchone()
        assert b["result"] == "unknown" and b["segments"] == 2 and b["rejoins"] == 1


def test_main_preflight_failure_records_and_zero_games_never_connects(tmp_path, monkeypatch):
    import sys

    cfg = config(local=True, stop={"minutes": None, "games": 0})
    path = tmp_path / "config.json"
    path.write_text(json.dumps(cfg))
    monkeypatch.setattr(ladder, "code_identity", lambda: {"commit": "test", "branch": "ladder-runner", "dirty": True})
    monkeypatch.setattr(ladder, "extension_identity", lambda: {"module": None, "native": [], "SEARCH": False})
    calls = []
    def preflight(bot, timing=None):
        calls.append(bot)
        return {"ok": False, "summary": "test preflight failure"}
    monkeypatch.setitem(sys.modules, "play.preflight", SimpleNamespace(run_preflight=preflight))
    arguments = ["--config", str(path), "--local", "--runs-dir", str(tmp_path / "runs"), "--db",
                 str(tmp_path / "ladder.sqlite"), "--run-id", "preflight-run"]
    assert asyncio.run(ladder.main(ladder.parse_args(arguments))) == 2
    assert len(calls) == 1
    m = json.loads((tmp_path / "runs" / "preflight-run" / "manifest.json").read_text())
    assert m["preflight"]["ok"] is False
    with connect_db(tmp_path) as conn:
        assert conn.execute("SELECT status FROM runs").fetchone()[0] == "config_error"
    arguments[-1] = "zero-games"
    assert asyncio.run(ladder.main(ladder.parse_args([*arguments, "--no-preflight"]))) == 0
    assert len(calls) == 1
    assert report_main(["--db", str(tmp_path / "ladder.sqlite"), "--run", "zero-games"]) == 0
    assert (tmp_path / "runs" / "zero-games" / "report.md").exists()


def test_main_resume_rejects_changed_checkpoint_before_loading(tmp_path, monkeypatch):
    checkpoint = tmp_path / "tiny.pt"
    checkpoint.write_bytes(b"first")
    cfg = config(local=True, bot={"factory": "training.player:DefaultBot", "checkpoint": str(checkpoint)})
    monkeypatch.setattr(ladder, "code_identity", lambda: {"commit": "test", "branch": "ladder-runner", "dirty": True})
    monkeypatch.setattr(ladder, "extension_identity", lambda: {"module": None, "native": [], "SEARCH": False})
    m = ladder.build_manifest(cfg, "resume-run")
    target = tmp_path / "runs" / "resume-run"
    target.mkdir(parents=True)
    (target / "manifest.json").write_text(json.dumps(m))
    checkpoint.write_bytes(b"changed")
    async def forbidden_load(worker):
        raise AssertionError("A changed checkpoint must be refused before loading")
    monkeypatch.setattr(BotWorker, "load", forbidden_load)
    args = ladder.parse_args(["--resume", "resume-run", "--runs-dir", str(tmp_path / "runs"), "--db",
                              str(tmp_path / "ladder.sqlite"), "--no-preflight"])
    assert asyncio.run(ladder.main(args)) == 2
    assert not (tmp_path / "ladder.sqlite").exists()


def test_git_identity_reads_real_worktree_without_changing_config():
    identity = ladder.code_identity()
    assert len(identity["commit"]) == 40
    assert identity["branch"]
    # A dirty checkout carries the hash of its diff, a clean one none.
    assert (identity["diff_sha256"] is not None) == identity["dirty"]
    assert identity["diff_sha256"] is None or len(identity["diff_sha256"]) == 64


def test_initial_species_before_side_and_late_decision_sequence(tmp_path):
    store = make_store(tmp_path)
    room = fake_room()
    room.side = None
    try:
        store.room_event(room, "message", {"text": "|switch|p2a: Z|Charizard, L80|100/100"})
        store.room_event(room, "request", {"seq": 1, "rqid": 1, "text": json.dumps(request_body())})
        room.side = "p1"
        store.room_event(room, "decision", decision())
        store.flush().result(timeout=5)
    finally:
        store.close()
    store = make_store(tmp_path)
    try:
        store.room_event(room, "decision", decision(source="fallback", fallback="deadline"))
        store.room_event(room, "late_decision", {"seq": 1, "rqid": 1, "choice": "move 1", "compute_ms": 800})
        store.flush().result(timeout=5)
    finally:
        store.close()
    with connect_db(tmp_path) as conn:
        b = conn.execute("SELECT * FROM battles").fetchone()
        assert json.loads(b["opp_species"]) == ["Charizard"]
    events = list(read_battle(tmp_path / "runs" / "test-run" / "battles" / f"{ROOM}.jsonl"))
    assert events[-1]["kind"] == "late_decision" and events[-1]["seq"] == 2


def test_gzip_reopens_and_appends_without_losing_history(tmp_path):
    store = make_store(tmp_path)
    room = fake_room()
    store.room_event(room, "message", {"text": "|turn|1"})
    store.room_event(room, "closed", {"ended": True, "stats": {}, "rated": True})
    store.flush().result(timeout=5)
    path = tmp_path / "runs" / "test-run" / "battles" / f"{ROOM}.jsonl"
    assert Path(str(path) + ".gz").exists()
    store.room_event(room, "message", {"text": "|turn|2"})
    store.room_event(room, "closed", {"ended": True, "stats": {}, "rated": True})
    store.flush().result(timeout=5)
    store.close()
    events = list(read_battle(path))
    assert [e["text"] for e in events if e["kind"] == "message"] == ["|turn|1", "|turn|2"]


@pytest.mark.parametrize("codes,limit,expected_starts", [([4, 0], 5, 2), ([1, 1], 1, 2), ([3], 5, 1), ([2], 5, 1)])
def test_supervisor_restart_resume_backoff_and_terminal_codes(tmp_path, monkeypatch, codes, limit, expected_starts):
    import os

    cfg = tmp_path / "config.json"
    cfg.write_text(json.dumps(config(local=True)))
    commands = []
    ticks = [0.0]
    def sleep(seconds):
        ticks[0] += seconds
    monkeypatch.setattr(supervise, "time", SimpleNamespace(time=time.time, monotonic=lambda: ticks[0], sleep=sleep))
    def popen(command, **options):
        code = codes[len(commands)]
        commands.append(command)
        if os.name == "nt":
            assert options["creationflags"] == supervise.subprocess.CREATE_NEW_PROCESS_GROUP
        target = tmp_path / "runs" / "supervised" / "manifest.json"
        target.write_text("{}")
        return SimpleNamespace(poll=lambda: code, returncode=code)
    monkeypatch.setattr(supervise.subprocess, "Popen", popen)
    args = ["--max-restarts", str(limit), "--", "--config", str(cfg), "--runs-dir", str(tmp_path / "runs"),
            "--db", str(tmp_path / "ladder.sqlite"), "--run-id", "supervised", "--local", "--no-preflight"]
    assert supervise.run(args) == codes[-1]
    assert len(commands) == expected_starts
    assert commands[0][-2:] == ["--run-id", "supervised"]
    if len(commands) > 1:
        assert commands[1][-2:] == ["--resume", "supervised"] and ticks[0] >= 3
    events = [json.loads(line) for line in (tmp_path / "runs" / "supervised" / "events.jsonl").read_text().splitlines()]
    assert sum(e["kind"] == "supervisor_start" for e in events) == expected_starts
    assert sum(e["kind"] == "supervisor_exit" for e in events) == expected_starts


def test_supervisor_ctrl_c_is_forwarded_and_never_restarts(tmp_path, monkeypatch):
    import os
    import signal

    cfg = tmp_path / "config.json"
    cfg.write_text(json.dumps(config(local=True)))
    handlers, forwarded, starts = {}, [], []
    def set_signal(sig, handler):
        old = handlers.get(sig, signal.SIG_DFL)
        handlers[sig] = handler
        return old
    monkeypatch.setattr(supervise.signal, "signal", set_signal)
    monkeypatch.setattr(supervise, "time", SimpleNamespace(time=time.time, monotonic=time.monotonic,
                                                         sleep=lambda s: None))
    class Child:
        returncode = None
        calls = 0
        def poll(self):
            self.calls += 1
            if self.calls == 1:
                handlers[signal.SIGINT](signal.SIGINT, None)
                return None
            return self.returncode
        def send_signal(self, sig):
            forwarded.append(sig)
            self.returncode = 0
    def popen(command, **options):
        starts.append(command)
        return Child()
    monkeypatch.setattr(supervise.subprocess, "Popen", popen)
    assert supervise.run(["--", "--config", str(cfg), "--runs-dir", str(tmp_path / "runs"),
                          "--run-id", "supervised", "--local"]) == 0
    assert forwarded == [signal.CTRL_BREAK_EVENT if os.name == "nt" else signal.SIGINT]
    assert len(starts) == 1


def test_snapshot_retries_wait_for_changed_count_and_never_guess(tmp_path, monkeypatch):
    async def scenario():
        requests = []
        sleeps = []
        snapshots = [{"elo": 1012, "w": 1, "l": 0}, {"elo": 1012, "w": 1, "l": 0},
                     {"elo": 1037, "w": 2, "l": 0}]
        async def handle(request):
            requests.append(request.path)
            r = snapshots[min(len(requests) - 1, len(snapshots) - 1)]
            return web.json_response({"ratings": {"gen9randomdoublesbattle": r}})
        app = web.Application()
        app.router.add_get("/users/{userid}.json", handle)
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", 0)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        cfg = config(users_json=f"http://127.0.0.1:{port}/users/{{userid}}.json")
        store = make_store(tmp_path, cfg)
        worker = BotWorker(cfg["bot"], DefaultBot(cfg["account"]))
        async def sleep(seconds):
            sleeps.append(seconds)
        # Patch only ladder's view of asyncio; aiohttp/websockets keep their real scheduler.
        monkeypatch.setattr(ladder, "asyncio", SimpleNamespace(sleep=sleep, Lock=asyncio.Lock))
        try:
            async with aiohttp.ClientSession() as session:
                client = LadderClient(worker, cfg, store, session=session)
                await client.capture_snapshot("start")
                await client.capture_snapshot("after_battle", ROOM, retry=True)
                assert len(requests) == 3 and client.last_snapshot["elo"] == 1037
                assert len(sleeps) == 1 and 4 <= sleeps[0] <= 5
                # Subsequent snapshots do not change w+l, so they must stay pending even when Elo exists.
                await client.capture_snapshot("after_battle", ROOM, retry=True)
                assert len(requests) == 8 and len(sleeps) == 5
            await asyncio.wrap_future(store.flush())
        finally:
            worker.shutdown()
            store.close()
            await runner.cleanup()
        events = [json.loads(line) for line in (store.run_dir / "events.jsonl").read_text().splitlines()]
        assert any(e["kind"] == "ratings_pending" for e in events)
        assert sum(e["kind"] == "ratings_snapshot" and e.get("pending") is False for e in events) == 2
    asyncio.run(scenario())


def test_first_signal_is_graceful_even_after_limit_then_second_exits(tmp_path, monkeypatch):
    import signal

    cfg = config(local=True, stop={"minutes": 0, "games": None})
    path = tmp_path / "config.json"
    path.write_text(json.dumps(cfg))
    monkeypatch.setattr(ladder, "code_identity", lambda: {"commit": "test", "branch": "ladder-runner", "dirty": True})
    monkeypatch.setattr(ladder, "extension_identity", lambda: {"module": None, "native": [], "SEARCH": False})
    exits = []
    monkeypatch.setattr(ladder.os, "_exit", exits.append)
    async def drive(client):
        client.request_stop("minutes limit")
        handler = signal.getsignal(signal.SIGINT)
        handler(signal.SIGINT, None)
        assert not client.force_stop and not exits
        handler(signal.SIGINT, None)
        assert client.force_stop and exits == [130]
        return 130
    monkeypatch.setattr(LadderClient, "drive", drive)
    arguments = ["--config", str(path), "--local", "--runs-dir", str(tmp_path / "runs"), "--db",
                 str(tmp_path / "ladder.sqlite"), "--run-id", "signal-run", "--no-preflight"]
    assert asyncio.run(ladder.main(ladder.parse_args(arguments))) == 130


def test_recorder_startup_write_failure_is_health_pause(tmp_path, monkeypatch):
    from play.runstore import RecorderError

    cfg = config(local=True, stop={"minutes": None, "games": 0})
    path = tmp_path / "config.json"
    path.write_text(json.dumps(cfg))
    monkeypatch.setattr(ladder, "code_identity", lambda: {"commit": "test", "branch": "ladder-runner", "dirty": True})
    monkeypatch.setattr(ladder, "extension_identity", lambda: {"module": None, "native": [], "SEARCH": False})
    def broken(*args):
        raise RecorderError("Write failure")
    monkeypatch.setattr(ladder, "RunStore", broken)
    assert asyncio.run(ladder.main(ladder.parse_args(["--config", str(path), "--local", "--no-preflight"]))) == 3


@pytest.mark.parametrize("values", [{"server": "http://127.0.0.1"}, {"timing": {"min_response_s": None}},
                                    {"stop": {"minutes": float("nan")}},
                                    {"bot": {"factory": "x:y", "password": PASSWORD}}])
def test_invalid_config_is_rejected_without_secrets(values):
    with pytest.raises(ladder.ConfigError) as raised:
        config(**values)
    assert PASSWORD not in str(raised.value)


def test_bad_battle_streak_includes_errors_before_rejoin_and_resets(tmp_path):
    async def scenario():
        cfg = config(local=True, stop={"minutes": None, "games": None})
        store = make_store(tmp_path, cfg)
        worker = BotWorker(cfg["bot"], DefaultBot(cfg["account"]))
        client = LadderClient(worker, cfg, store)
        try:
            for i in range(3):
                room = fake_room()
                room.room = f"battle-gen9randomdoublesbattle-{i + 1}"
                room.stats = {"errors": 0, "refused": 0}
                store.emit("battle_open", {}, room.room)
                client.room_event(room, "closed", {"ended": False, "stats": {"errors": 1}, "rated": True})
                # The later, rejoined segment has no errors. The whole battle still had a technical problem.
                store.emit("battle_open", {"rejoined": True}, room.room)
                room._ladder_segment = 2
                client.room_event(room, "end", {"won": True, "result": {"winner": cfg["account"]},
                                                "end_reason": "normal", "turns": 1})
                client.room_event(room, "closed", {"ended": True, "stats": {"errors": 0}, "rated": True})
                await client.after_battle(room)
            assert client.exit_code == 3 and client.stop_requested and client.bad_battles == 3
            await asyncio.wrap_future(store.flush())
        finally:
            worker.shutdown()
            store.close()
        with connect_db(tmp_path) as conn:
            assert [r[0] for r in conn.execute("SELECT errors FROM battles ORDER BY room")] == [1, 1, 1]
    asyncio.run(scenario())


def test_resume_keeps_pre_battle_official_rating_baseline(tmp_path):
    cfg = config(local=True)
    store = make_store(tmp_path, cfg)
    store.emit("ratings_snapshot", {"rating": {"elo": 1012, "w": 1, "l": 0}, "time": time.time() - 1})
    store.emit("battle_open", {}, ROOM)
    store.flush().result(timeout=5)
    store.close()
    store = make_store(tmp_path, cfg)
    worker = BotWorker(cfg["bot"], DefaultBot(cfg["account"]))
    try:
        client = LadderClient(worker, cfg, store)
        client.last_snapshot = {"elo": 1037, "w": 2, "l": 0}
        assert client.battle_baselines[ROOM]["w"] == 1
        assert client.battle_baselines[ROOM]["elo"] == 1012
    finally:
        worker.shutdown()
        store.close()


@pytest.mark.parametrize("games", [1, 2])
def test_full_ladder_rating_wait_and_stop(tmp_path, games):
    sent = asyncio.run(fake_ladder_run(tmp_path, finish=True, games=games))
    assert sum(text == "|/search gen9randomdoublesbattle" for text in sent) == games
    with connect_db(tmp_path) as conn:
        b = conn.execute("SELECT * FROM battles").fetchone()
        assert b["result"] == "win" and b["rated"] == 1 and b["our_rating_after_room"] == 1037
        assert b["opp_rating_before"] == 1234 and b["timer_sent"]


def test_full_ladder_reconnect_pending_rejoin(tmp_path):
    sent = asyncio.run(fake_ladder_run(tmp_path, finish=True, reconnect=True))
    assert sum(text == "|/search gen9randomdoublesbattle" for text in sent) == 1
    with connect_db(tmp_path) as conn:
        b = conn.execute("SELECT * FROM battles").fetchone()
        assert b["result"] == "win" and b["segments"] == 2 and b["rejoins"] == 1


def test_snapshots_count_from_a_new_account_and_absorb_late_updates(tmp_path, monkeypatch):
    """A fresh account has no rating in the format at the start (0 games); a late snapshot that already counts two
    battles confirms both instead of leaving the second pending."""
    async def scenario():
        replies = [{"userid": "numbrionladder", "ratings": {}},
                   {"userid": "numbrionladder", "ratings": {"gen9randomdoublesbattle": {"elo": 1040, "w": 2, "l": 0}}}]
        served = []

        async def handle(request):
            served.append(request.path)
            return web.json_response(replies[min(len(served) - 1, len(replies) - 1)])
        app = web.Application()
        app.router.add_get("/users/{userid}.json", handle)
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", 0)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        cfg = config(users_json=f"http://127.0.0.1:{port}/users/{{userid}}.json")
        store = make_store(tmp_path, cfg)
        worker = BotWorker(cfg["bot"], DefaultBot(cfg["account"]))

        async def sleep(seconds):
            pass
        monkeypatch.setattr(ladder, "asyncio", SimpleNamespace(sleep=sleep, Lock=asyncio.Lock))
        try:
            async with aiohttp.ClientSession() as session:
                client = LadderClient(worker, cfg, store, session=session)
                await client.capture_snapshot("start")
                assert client.base_count == 0
                await client.capture_snapshot("after_battle", "battle-a", retry=True, expected=1)
                await client.capture_snapshot("after_battle", "battle-b", retry=True, expected=2)
                assert len(served) == 3  # one fetch each after the start: both already counted
            await asyncio.wrap_future(store.flush())
        finally:
            worker.shutdown()
            store.close()
            await runner.cleanup()
        events = [json.loads(line) for line in (store.run_dir / "events.jsonl").read_text().splitlines()]
        assert not any(e["kind"] == "ratings_pending" for e in events)
    asyncio.run(scenario())
