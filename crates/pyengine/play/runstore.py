"""Complete, flushed JSONL records and derived SQLite data, owned by one queued writer thread."""
from __future__ import annotations

import copy
import gzip
import hashlib
import json
import logging
import queue
import re
import sqlite3
import threading
import time
from concurrent.futures import Future
from pathlib import Path

from play.auth import redact_traffic

log = logging.getLogger("play")
RELIABILITY = {"refused", "server_error", "fallback", "late_decision", "deadline_miss", "unhealthy",
               "disconnect", "reconnect", "rejoin", "auth_fail", "popup", "recorder_error", "pause", "stop"}
SCHEMA = """
CREATE TABLE IF NOT EXISTS runs (
 run_id TEXT PRIMARY KEY, account TEXT, server TEXT, format TEXT, started REAL, ended REAL, status TEXT,
 manifest TEXT);
CREATE TABLE IF NOT EXISTS battles (
 run_id TEXT, room TEXT, rated INTEGER DEFAULT 0, side TEXT, opponent TEXT, our_rating_before REAL,
 opp_rating_before REAL, our_rating_after_room REAL, rating_delta_room REAL, result TEXT, end_reason TEXT,
 started REAL, ended REAL, turns INTEGER DEFAULT 0, decisions INTEGER DEFAULT 0,
 search INTEGER DEFAULT 0, policy INTEGER DEFAULT 0, fallback INTEGER DEFAULT 0, "default" INTEGER DEFAULT 0,
 refused INTEGER DEFAULT 0, errors INTEGER DEFAULT 0, rejoins INTEGER DEFAULT 0, segments INTEGER DEFAULT 0,
 timer_sent REAL, timer_on REAL, queue_wait_s REAL, our_species TEXT DEFAULT '[]', opp_species TEXT DEFAULT '[]',
 tera_used INTEGER DEFAULT 0, tera_turn INTEGER, replay_url TEXT, log_path TEXT, PRIMARY KEY (run_id, room));
CREATE TABLE IF NOT EXISTS decisions (
 run_id TEXT, room TEXT, seq INTEGER, request_seq INTEGER, rqid INTEGER, turn INTEGER, source TEXT, fallback TEXT,
 choice TEXT, a0 INTEGER, a1 INTEGER, win_probability REAL, value REAL, worlds INTEGER, cells INTEGER,
 policy_choice TEXT, agrees_with_policy INTEGER, received REAL, submitted REAL, total_ms REAL, compute_ms REAL,
 queue_ms REAL, paced_ms REAL, budget_ms REAL, deadline REAL, clock_turn_left REAL, clock_total_left REAL, note TEXT,
 PRIMARY KEY (run_id, room, seq));
CREATE TABLE IF NOT EXISTS ratings (
 run_id TEXT, time REAL, source TEXT, room TEXT, elo REAL, gxe REAL, rpr REAL, rprd REAL, w INTEGER, l INTEGER,
 delta REAL, raw TEXT);
CREATE TABLE IF NOT EXISTS events (run_id TEXT, time REAL, kind TEXT, room TEXT, detail TEXT);
CREATE TABLE IF NOT EXISTS resources (
 run_id TEXT, time REAL, cpu REAL, proc_cpu REAL, rss_mb REAL, mem_percent REAL, worker_queue INTEGER, rooms INTEGER);
CREATE INDEX IF NOT EXISTS events_run ON events(run_id, kind);
CREATE INDEX IF NOT EXISTS ratings_run ON ratings(run_id, time);
"""


class RecorderError(OSError):
    """Run files or the shared database could not be written."""


def canonical(value) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, default=str)


def json_hash(value) -> str:
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def safe_id(value: str) -> str:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,180}", value):
        raise ValueError("Run and room ids must contain only letters, numbers, underscores and hyphens")
    return value


def read_battle(path: Path):
    """Read plain and gzip logs, including concatenated gzip members appended after a rejoin."""
    if path.suffix == ".gz" and not path.exists():
        path = path.with_suffix("")
    elif not path.exists():
        path = Path(str(path) + ".gz")
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rt", encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                yield json.loads(line)


class RunStore:
    """emit and room_event only queue work. failed is sticky; callers must stop new matchmaking on failure."""

    def __init__(self, runs_dir: Path, db: Path, manifest: dict, secrets: tuple[str, ...] = ()):
        self.run_id = safe_id(manifest["run_id"])
        self.run_dir = Path(runs_dir) / self.run_id
        self.db_path = Path(db)
        self.manifest = manifest
        self.secrets = list(secrets)
        self.failed = threading.Event()
        self.failure: str | None = None
        self.queue: queue.Queue = queue.Queue()
        self.ready = threading.Event()
        self.initial_battles: dict[str, dict] = {}
        self.initial_decisions: dict[str, list[dict]] = {}
        self.initial_baselines: dict[str, dict | None] = {}
        self._decision_seq: dict[str, int] = {}
        self._sent_requests: dict[tuple[str, int], int] = {}
        self._species_seen: dict[str, dict[str, list[str]]] = {}
        self._closed_errors: dict[tuple[str, int], int] = {}
        self._closed = False
        self.thread = threading.Thread(target=self._run, name="ladder-writer", daemon=True)
        self.thread.start()
        self.ready.wait()  # startup, before any websocket or event-loop work
        if self.failed.is_set():
            raise RecorderError("Could not initialize the ladder recorder")
        self._decision_seq = {room: row["decisions"] for room, row in self.initial_battles.items()}

    def add_secret(self, secret: str) -> None:
        if secret:
            self.secrets.append(secret)

    def clean(self, value):
        if isinstance(value, str):
            value = redact_traffic(value)
            for secret in tuple(self.secrets):
                value = value.replace(secret, "[redacted]")
            return value
        if isinstance(value, dict):
            return {str(k): "[redacted]" if str(k).lower() in {"password", "pass", "assertion", "challstr"}
                    else self.clean(v) for k, v in value.items()}
        if isinstance(value, (list, tuple)):
            return [self.clean(v) for v in value]
        return value

    def emit(self, kind: str, data: dict | None = None, room: str | None = None) -> None:
        if self._closed:
            return
        if room:
            safe_id(room)
        data = copy.deepcopy(data or {})
        event = {**data, "t": data.get("time", time.time()), "kind": kind, "room": room}
        self.queue.put_nowait(("event", self.clean(event), None))

    def room_event(self, room, kind: str, data: dict) -> None:
        data = dict(data)
        data["battle"] = {key: copy.deepcopy(getattr(room, key, None)) for key in
                          ("side", "turn", "players", "rated", "started", "rejoined", "timer_on")}
        data["battle"]["segment"] = getattr(room, "_ladder_segment", None)
        if kind == "decision":
            seq = self._decision_seq.get(room.room, 0) + 1
            self._decision_seq[room.room] = seq
            data["request_seq"] = data.get("seq")
            self._sent_requests[(room.room, data.get("seq", 0))] = seq
            data["seq"] = seq
        elif kind == "late_decision":
            data["request_seq"] = data.get("seq")
            data["seq"] = self._sent_requests.get((room.room, data.get("seq", 0)), data.get("seq"))
        self.emit(kind, data, room.room)

    def finish(self, status: str) -> Future:
        future: Future = Future()
        self.queue.put_nowait(("finish", status, future))
        return future

    def flush(self) -> Future:
        future: Future = Future()
        self.queue.put_nowait(("barrier", None, future))
        return future

    def close(self) -> None:
        if not self._closed:
            self._closed = True
            self.queue.put_nowait(("close", None, None))
            self.thread.join(timeout=20)
            if self.thread.is_alive():
                self.failed.set()
                self.failure = "Recorder did not finish within 20 seconds"

    def _run(self) -> None:
        handles = {}
        conn = None
        try:
            (self.run_dir / "battles").mkdir(parents=True, exist_ok=True)
            self.db_path.parent.mkdir(parents=True, exist_ok=True)
            conn = sqlite3.connect(self.db_path, timeout=20)
            conn.row_factory = sqlite3.Row
            conn.execute("PRAGMA journal_mode=WAL")
            conn.executescript(SCHEMA)
            manifest = self.clean(self.manifest)
            tmp = self.run_dir / "manifest.json.tmp"
            tmp.write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")
            tmp.replace(self.run_dir / "manifest.json")
            conn.execute("INSERT INTO runs VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(run_id) DO UPDATE SET "
                         "status='running', ended=NULL, manifest=excluded.manifest",
                         (self.run_id, manifest["account"], manifest["server"], manifest["format"],
                          manifest["started"], None, "running", canonical(manifest)))
            self.initial_battles = {r["room"]: dict(r) for r in conn.execute(
                "SELECT * FROM battles WHERE run_id=?", (self.run_id,))}
            for decision in conn.execute("SELECT * FROM decisions WHERE run_id=? ORDER BY room,seq", (self.run_id,)):
                self.initial_decisions.setdefault(decision["room"], []).append(dict(decision))
            for room, battle in self.initial_battles.items():
                rating = conn.execute("SELECT w,l,elo FROM ratings WHERE run_id=? AND source='users_json' "
                                      "AND time<=? ORDER BY time DESC LIMIT 1",
                                      (self.run_id, battle["started"])).fetchone()
                self.initial_baselines[room] = dict(rating) if rating else None
            # A previous graceful shutdown labelled unresolved rooms unknown; a resume may see them again.
            conn.execute("UPDATE battles SET result=NULL WHERE run_id=? AND result='unknown' AND ended IS NULL",
                         (self.run_id,))
            for row in self.initial_battles.values():
                if row["result"] == "unknown" and row["ended"] is None:
                    row["result"] = None
            conn.commit()
        except Exception as error:
            self._fail(error)
        finally:
            self.ready.set()
        if self.failed.is_set():
            if conn:
                conn.close()
            return
        while True:
            command, data, future = self.queue.get()
            try:
                if command == "close":
                    break
                if command == "event":
                    event = data
                    path = self.run_dir / "events.jsonl"
                    late = None
                    if event["room"]:
                        path = self.run_dir / "battles" / f"{event['room']}.jsonl"
                        gz = Path(str(path) + ".gz")
                        if gz.exists() and not path.exists():
                            late = gz  # a finished, compressed battle (leaving the room, its rating snapshot)
                    if late is not None:
                        # One more gzip member per late event; read_battle reads concatenated members.
                        with gzip.open(late, "at", encoding="utf-8", newline="\n") as handle:
                            handle.write(canonical(event) + "\n")
                        self._project(conn, event, late)
                    else:
                        handle = handles.get(path)
                        if handle is None:
                            handles[path] = handle = path.open("a", encoding="utf-8", newline="\n")
                        handle.write(canonical(event) + "\n")
                        handle.flush()
                        self._project(conn, event, path)
                    conn.commit()
                    if late is None and event["kind"] == "closed" and event.get("ended"):
                        handles.pop(path).close()
                        self._compress(conn, path, event["room"])
                elif command == "finish":
                    conn.execute("UPDATE runs SET ended=?,status=? WHERE run_id=?", (time.time(), data, self.run_id))
                    conn.execute("UPDATE battles SET result='unknown' WHERE run_id=? AND result IS NULL",
                                 (self.run_id,))
                    conn.commit()
                if future:
                    future.set_result(None)
            except Exception as error:
                conn.rollback()
                self._fail(error)
                if future:
                    future.set_exception(OSError(self.failure))
        for handle in handles.values():
            handle.close()
        conn.close()

    def _fail(self, error: Exception) -> None:
        self.failure = f"Recorder write failed ({type(error).__name__})"
        self.failed.set()
        log.error("%s", self.failure)

    def _compress(self, conn, path: Path, room: str) -> None:
        tmp, dest = Path(str(path) + ".gz.tmp"), Path(str(path) + ".gz")
        expected = hashlib.sha256()
        with path.open("rb") as source, gzip.open(tmp, "wb") as target:
            while chunk := source.read(1024 * 1024):
                expected.update(chunk)
                target.write(chunk)
        actual = hashlib.sha256()
        with gzip.open(tmp, "rb") as source:
            while chunk := source.read(1024 * 1024):
                actual.update(chunk)
        if actual.digest() != expected.digest():
            raise OSError("Compressed battle log failed verification")
        tmp.replace(dest)
        path.unlink()
        conn.execute("UPDATE battles SET log_path=? WHERE run_id=? AND room=?", (str(dest), self.run_id, room))
        conn.commit()

    def _update(self, conn, room, values):
        if values:
            columns = ",".join(f'"{key}"=?' for key in values)
            conn.execute(f"UPDATE battles SET {columns} WHERE run_id=? AND room=?",
                         (*values.values(), self.run_id, room))

    def _project(self, conn, e, path):
        room, kind, now = e["room"], e["kind"], e["t"]
        if kind in RELIABILITY:
            conn.execute("INSERT INTO events VALUES (?,?,?,?,?)", (self.run_id, now, kind, room, canonical(e)))
        if kind == "resources":
            conn.execute("INSERT INTO resources VALUES (?,?,?,?,?,?,?,?)",
                         (self.run_id, now, *(e.get(k) for k in
                          ("cpu", "proc_cpu", "rss_mb", "mem_percent", "worker_queue", "rooms"))))
        if kind == "ratings_snapshot" and e.get("rating"):
            r = e["rating"]
            conn.execute("INSERT INTO ratings VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                         (self.run_id, now, "users_json", e.get("battle_room"), *(r.get(k) for k in
                          ("elo", "gxe", "rpr", "rprd", "w", "l")), None, canonical(e.get("raw", r))))
        if not room:
            return
        conn.execute("INSERT OR IGNORE INTO battles(run_id,room,started,log_path) VALUES (?,?,?,?)",
                     (self.run_id, room, now, str(path)))
        row = dict(conn.execute("SELECT * FROM battles WHERE run_id=? AND room=?", (self.run_id, room)).fetchone())
        meta = e.get("battle") or {}
        side = meta.get("side") or row["side"]
        values = {"log_path": str(path)}
        if side:
            values["side"] = side
        if meta.get("rated"):
            values["rated"] = 1
        if meta.get("turn"):
            values["turns"] = max(row["turns"], meta["turn"])
        players = meta.get("players") or e.get("players") or {}
        if side and players:
            ours, opp = players.get(side, {}), players.get("p2" if side == "p1" else "p1", {})
            # A rejoined segment's |player| lines may lack the ratings: keep the first ones seen.
            values.update(opponent=opp.get("name") or row["opponent"],
                          our_rating_before=row["our_rating_before"] if row["our_rating_before"] is not None
                          else ours.get("rating"),
                          opp_rating_before=row["opp_rating_before"] if row["opp_rating_before"] is not None
                          else opp.get("rating"))
        if kind == "battle_open":
            values.update(segments=row["segments"] + 1, rejoins=row["rejoins"] + int(e.get("rejoined", False)))
            if e.get("queue_wait_s") is not None:
                values["queue_wait_s"] = e["queue_wait_s"]
        elif kind == "timer_sent":
            values["timer_sent"] = row["timer_sent"] or now
        elif kind == "timer" and e.get("state") == "on":
            values["timer_on"] = now
        elif kind == "request":
            try:
                request = json.loads(e["text"]) if isinstance(e.get("text"), str) else e.get("request", {})
                side = request.get("side", {}).get("id") or side
                if side:
                    values["side"] = side
                if not json.loads(row["our_species"]):
                    values["our_species"] = canonical([p.get("details", "").split(",")[0]
                                                       for p in request.get("side", {}).get("pokemon", [])])
            except (ValueError, AttributeError):
                pass
        elif kind == "message":
            by_side = self._species_seen.setdefault(room, {"p1": [], "p2": []})
            for line in e.get("text", "").splitlines():
                parts = line.split("|")
                if len(parts) > 3 and parts[1] in {"switch", "drag", "replace"}:
                    shown_side, species = parts[2][:2], parts[3].split(",")[0]
                    if shown_side in by_side and species not in by_side[shown_side]:
                        by_side[shown_side].append(species)
        elif kind == "decision":
            a, timing = e.get("analysis") or {}, e.get("timing") or {}
            source = e.get("source") or a.get("source") or "default"
            policy = a.get("policy") or {}
            policy_choice = policy.get("choice")
            clock = e.get("clock") or {}
            if not clock:
                clock = e.get("battle_clock") or {}
            cols = ["run_id", "room", "seq", "request_seq", "rqid", "turn", "source", "fallback", "choice",
                    "a0", "a1", "win_probability", "value", "worlds", "cells", "policy_choice", "agrees_with_policy",
                    "received", "submitted", "total_ms", "compute_ms", "queue_ms", "paced_ms", "budget_ms",
                    "deadline", "clock_turn_left", "clock_total_left", "note"]
            vals = [self.run_id, room, e["seq"], e.get("request_seq"), e.get("rqid"), e.get("turn"), source,
                    e.get("fallback"), e.get("choice"), e.get("a0"), e.get("a1"), a.get("win_probability"),
                    a.get("value"), a.get("worlds"), a.get("cells"), policy_choice,
                    int(policy_choice == e.get("choice")) if policy_choice is not None else None,
                    *(timing.get(k) for k in ("received", "submitted", "total_ms", "compute_ms", "queue_ms",
                                              "paced_ms", "budget_ms", "deadline")),
                    clock.get("turn_left"), clock.get("total_left"), a.get("note")]
            conn.execute(f"INSERT INTO decisions ({','.join(cols)}) VALUES ({','.join('?' for _ in cols)})", vals)
            values["decisions"] = row["decisions"] + 1
            if source in {"search", "policy", "fallback", "default"}:
                values[source] = row[source] + 1
            if "terastallize" in (e.get("choice") or ""):
                values.update(tera_used=1, tera_turn=row["tera_turn"] or e.get("turn"))
            if e.get("fallback") or source == "fallback":
                conn.execute("INSERT INTO events VALUES (?,?,?,?,?)",
                             (self.run_id, now, "fallback", room, canonical(e)))
            deadline_s = self.manifest.get("timing", {}).get("deadline_s")
            if deadline_s and timing.get("total_ms", 0) > deadline_s * 1000:
                conn.execute("INSERT INTO events VALUES (?,?,?,?,?)",
                             (self.run_id, now, "deadline_miss", room, canonical(e)))
        elif kind == "refused":
            values["refused"] = row["refused"] + 1
        elif kind == "rating" and e.get("userid") == re.sub(r"[^a-z0-9]", "", self.manifest["account"].lower()):
            values.update(our_rating_after_room=e.get("new"), rating_delta_room=e.get("delta"))
            conn.execute("INSERT INTO ratings VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                         (self.run_id, now, "room", room, e.get("new"), None, None, None, None, None,
                          e.get("delta"), canonical(e)))
        elif kind == "end":
            result = e.get("result") or {}
            outcome = "tie" if result.get("tie") else "win" if e.get("won") is True else \
                      "loss" if e.get("won") is False else "unknown"
            values.update(result=outcome, ended=now, end_reason=e.get("end_reason"), turns=e.get("turns", 0))
        elif kind == "closed":
            key = (room, meta.get("segment") or row["segments"])
            errors = (e.get("stats") or {}).get("errors", 0)
            values["errors"] = row["errors"] + errors - self._closed_errors.get(key, 0)
            self._closed_errors[key] = errors
            values["rated"] = int(bool(e.get("rated") or meta.get("rated") or row["rated"]))
            if meta.get("timer_on"):
                values["timer_on"] = meta["timer_on"]
        elif kind == "replay":
            values["replay_url"] = e.get("url")
        # Initial switches can precede the request that identifies our side. Retain both sides until then.
        if side:
            seen = json.loads(row["opp_species"])
            for species in self._species_seen.get(room, {}).get("p2" if side == "p1" else "p1", []):
                if species not in seen:
                    seen.append(species)
            values["opp_species"] = canonical(seen)
        self._update(conn, room, values)
