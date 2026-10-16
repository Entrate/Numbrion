"""One frozen bot, one registered Showdown account, one ladder battle at a time.

    python crates/pyengine/play/ladder.py --config crates/pyengine/play/ladder.json --minutes 60

The observer contract in driver.py is the source of battle results. Losing a websocket never declares a loss.
"""
from __future__ import annotations

import argparse
import asyncio
import ctypes
import hashlib
import importlib.metadata
import json
import logging
import math
import os
import platform
import re
import signal
import socket
import subprocess
import sys
import tempfile
import time
from collections import Counter
from pathlib import Path
from urllib.parse import urlsplit

HERE = Path(__file__).resolve().parent
if sys.path and Path(sys.path[0] or ".").resolve() == HERE:
    sys.path[0] = str(HERE.parent)
elif str(HERE.parent) not in sys.path:
    sys.path.insert(0, str(HERE.parent))
REPO = HERE.parents[2]

import aiohttp  # noqa: E402
import psutil  # noqa: E402
import websockets  # noqa: E402

from play.auth import AuthError, Credentials, load_credentials, login_assertion  # noqa: E402
from play.client import ShowdownClient  # noqa: E402
from play.driver import BotLog, BotWorker, DecisionTiming, to_id  # noqa: E402
from play.runstore import RecorderError, RunStore, json_hash, safe_id  # noqa: E402

log = logging.getLogger("play")
DEFAULTS = {"account": "numbrion", "server": "wss://sim3.psim.us/showdown/websocket",
            "format": "gen9randomdoublesbattle", "credentials": None,
            "timing": {"search_s": 3.0, "deadline_s": 6.0, "min_response_s": 2.0},
            "rating_wait_s": 30, "stop": {"minutes": 60, "games": None}, "save_replays": False,
            "users_json": "https://pokemonshowdown.com/users/{userid}.json"}


SEARCH_CONFIRM_S = 15.0  # how long a /search may go unconfirmed before the runner searches again


class ConfigError(ValueError):
    pass


class KeepAwake:
    """Prevent idle sleep on Windows and macOS while the runner is active."""

    def __init__(self):
        self.caffeinate = None

    def __enter__(self):
        if os.name == "nt":
            ctypes.windll.kernel32.SetThreadExecutionState(0x80000000 | 0x1)
        elif sys.platform == "darwin":
            try:
                self.caffeinate = subprocess.Popen(["caffeinate", "-i", "-w", str(os.getpid())])
            except OSError as error:
                log.warning("Could not start caffeinate; idle sleep is still enabled: %s", error)
        return self

    def __exit__(self, *args):
        if os.name == "nt":
            ctypes.windll.kernel32.SetThreadExecutionState(0x80000000)
        if self.caffeinate is not None:
            try:
                self.caffeinate.terminate()
                self.caffeinate.wait()
            except OSError as error:
                log.warning("Could not stop caffeinate: %s", error)
            self.caffeinate = None


def absolute_path(value: str | Path) -> Path:
    path = Path(value).expanduser()
    return path.resolve() if path.is_absolute() else (REPO / path).resolve()


def load_config(path: Path | None, local: bool = False, base: dict | None = None) -> dict:
    config = json.loads(json.dumps(DEFAULTS))
    supplied = base if base is not None else json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(supplied, dict):
        raise ConfigError("Config must be a JSON object")
    for key, value in supplied.items():
        if key in {"stop", "timing"} and isinstance(value, dict):
            config[key].update(value)
        else:
            config[key] = value
    def contains_secret(value):
        if isinstance(value, dict):
            return any(str(k).lower() in {"password", "pass", "assertion", "challstr"} or contains_secret(v)
                       for k, v in value.items())
        return any(contains_secret(v) for v in value) if isinstance(value, list) else False
    if contains_secret(config):
        raise ConfigError("Use a separate credentials file or environment variables for secrets")
    if not isinstance(config.get("bot"), dict) or not config["bot"].get("factory"):
        raise ConfigError("Config needs bot.factory")
    account = config["account"]
    if not isinstance(account, str) or not to_id(account) or len(account) > 18 or any(c in account for c in ",|\r\n"):
        raise ConfigError("Account must be a valid Showdown name of at most 18 characters")
    if config["format"] != "gen9randomdoublesbattle":
        raise ConfigError("This runner only supports gen9randomdoublesbattle")
    if not isinstance(config["server"], str):
        raise ConfigError("server must be a websocket URL")
    server = urlsplit(config["server"])
    if server.scheme not in {"ws", "wss"} or not server.netloc:
        raise ConfigError("server must be a ws:// or wss:// URL")
    if not isinstance(config["stop"], dict) or not isinstance(config["timing"], dict):
        raise ConfigError("stop and timing must be objects")
    for key in ("minutes", "games"):
        value = config["stop"].get(key)
        if value is not None and (not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0):
            raise ConfigError(f"stop.{key} must be nonnegative or null")
    if config["stop"].get("games") is not None and int(config["stop"]["games"]) != config["stop"]["games"]:
        raise ConfigError("stop.games must be an integer")
    for key, value in config["timing"].items():
        if key not in DecisionTiming.__dataclass_fields__ or (value is not None and
                (not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0)):
            raise ConfigError(f"Invalid timing.{key}")
        if value is None and key not in {"search_s", "deadline_s"}:
            raise ConfigError(f"timing.{key} cannot be null")
    if (not isinstance(config["rating_wait_s"], (int, float)) or not math.isfinite(config["rating_wait_s"])
            or config["rating_wait_s"] < 0):
        raise ConfigError("rating_wait_s must be nonnegative")
    spec = config["bot"]
    spec["name"] = account
    if spec.get("checkpoint"):
        spec["checkpoint"] = str(absolute_path(spec["checkpoint"]))
    pools = spec.get("pools")
    if not pools and spec["factory"] == "training.search:SearchBot" and spec.get("search") is not False:
        pools = ["data/teams/pool-s1-200k.txt.gz"]
    if pools is not None:
        spec["pools"] = [str(absolute_path(p)) for p in pools]
    if config.get("numbrion_path"):
        config["numbrion_path"] = str(absolute_path(config["numbrion_path"]))
    if local or config.get("local"):
        config["users_json"] = None
        if config["server"] == DEFAULTS["server"]:
            config["server"] = "ws://localhost:8000/showdown/websocket"
    config["local"] = bool(local or config.get("local"))
    return config


def file_identity(path: str | Path) -> dict:
    path = Path(path).resolve()
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return {"path": str(path), "size": path.stat().st_size, "sha256": digest.hexdigest()}


def code_identity() -> dict:
    # Older Git ignores safe.directory from -c. A temporary global config trusts only this worktree;
    # the user's global config is never changed. This also works when the sandbox has a different owner.
    with tempfile.TemporaryDirectory(prefix="numbrion-git-") as directory:
        config_path = Path(directory) / "gitconfig"
        config_path.write_text(f'[safe]\n\tdirectory = "{REPO.as_posix()}"\n', encoding="utf-8")
        env = {**os.environ, "GIT_CONFIG_GLOBAL": str(config_path)}
        def git(*args):
            return subprocess.check_output(["git", *args], cwd=REPO, env=env, stderr=subprocess.DEVNULL)
        try:
            commit = git("rev-parse", "HEAD").decode().strip()
            branch = git("branch", "--show-current").decode().strip()
            dirty = bool(git("status", "--porcelain"))
            diff = hashlib.sha256(git("diff", "--no-ext-diff", "HEAD")).hexdigest() if dirty else None
            return {"commit": commit, "branch": branch, "dirty": dirty, "diff_sha256": diff}
        except (OSError, subprocess.CalledProcessError):
            raise ConfigError("Cannot read git identity for a frozen ladder run") from None


def extension_identity() -> dict:
    try:
        import numbrion
    except ImportError:
        return {"module": None, "native": [], "SEARCH": None}
    path = Path(numbrion.__file__).resolve()
    native = {p for pattern in ("*.pyd", "*.so") for p in path.parent.glob(pattern)}
    if path.suffix in {".pyd", ".so"}:
        native.add(path)
    return {"module": str(path), "native": [file_identity(p) for p in sorted(native)],
            "SEARCH": getattr(numbrion, "SEARCH", None)}


def hardware_identity() -> dict:
    def version(package):
        try:
            return importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError:
            return None
    return {"cpu": os.getenv("PROCESSOR_IDENTIFIER") or platform.processor(),
            "logical_cores": psutil.cpu_count(), "physical_cores": psutil.cpu_count(logical=False),
            "ram_bytes": psutil.virtual_memory().total, "os": platform.platform(), "hostname": socket.gethostname(),
            "python": platform.python_version(), "torch": version("torch"), "numpy": version("numpy")}


def make_run_id(account: str, commit: str | None = None) -> str:
    short = (commit or code_identity()["commit"])[:8]
    return f"{time.strftime('%Y%m%d-%H%M%S')}-{to_id(account)}-{short}"


def build_manifest(config: dict, run_id: str, started: float | None = None) -> dict:
    spec = config["bot"]
    return {"run_id": safe_id(run_id), "account": config["account"], "server": config["server"],
            "format": config["format"], "started": started if started is not None else time.time(),
            "config": config, "config_sha256": json_hash(config), "code": code_identity(),
            "bot": {"spec": spec, "checkpoint": file_identity(spec["checkpoint"]) if spec.get("checkpoint") else None},
            "pools": [file_identity(p) for p in spec.get("pools", [])], "extension": extension_identity(),
            "seeds": {"search": spec["search"].get("seed", 0) if isinstance(spec.get("search"), dict)
                      else None if spec.get("search") is False else 0},
            "timing": config["timing"], "hardware": hardware_identity(), "preflight": None}


def validate_resume(previous: dict, current: dict, force: bool = False) -> list[str]:
    differences = []
    for label, old, new in (("config", previous.get("config_sha256"), current.get("config_sha256")),
                            ("checkpoint", previous["bot"].get("checkpoint"), current["bot"].get("checkpoint")),
                            ("pools", previous.get("pools"), current.get("pools")),
                            ("extension", previous.get("extension"), current.get("extension")),
                            ("git commit", previous["code"].get("commit"), current["code"].get("commit"))):
        if old != new:
            differences.append(label)
    if differences and not force:
        raise ConfigError("Frozen run differs in " + ", ".join(differences) + "; use a new run or explicit --force")
    return differences


def percentile(values, q: float) -> float:
    ordered = sorted(v for v in values if isinstance(v, (int, float)))
    if not ordered:
        return 0.0
    index = (len(ordered) - 1) * q
    lo, hi = int(index), min(len(ordered) - 1, int(index) + 1)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (index - lo)


def games(rating: dict) -> int:
    """Official games of a users-JSON rating entry (wins, losses and ties when listed)."""
    return int(rating.get("w", 0) or 0) + int(rating.get("l", 0) or 0) + int(rating.get("t", 0) or 0)


class LadderClient(ShowdownClient):
    """Reject challenges, reconcile running games, and keep all reconnect segments in the same record."""

    def __init__(self, worker: BotWorker, config: dict, store: RunStore, credentials: Credentials | None = None,
                 session: aiohttp.ClientSession | None = None):
        super().__init__(worker, config["server"], BotLog(), None, timer=True,
                         room_options={"observer": self, "timing": DecisionTiming.from_dict(config["timing"]),
                                       "rating_wait_s": config["rating_wait_s"]})
        self.config, self.store, self.credentials, self.session = config, store, credentials, session
        self.searching: set[str] = set()
        self.search_started: float | None = None
        self.reconcile_at = float("inf")
        self.auth_failures = 0
        self.next_auth_at = 0.0
        self.login_task: asyncio.Task | None = None
        self.login_sent_at: float | None = None
        self.stop_requested = False
        self.force_stop = False
        self.exit_code = 0
        self.stuck_since: float | None = None
        self.finished_count = sum(r["ended"] is not None for r in store.initial_battles.values())
        self.ended_rooms = {r["room"] for r in store.initial_battles.values() if r["ended"] is not None}
        self.done_rooms.update(self.ended_rooms)
        self.games.update(r["room"] for r in store.initial_battles.values() if r["result"] is None)
        self.summaries: dict[str, dict] = {}
        self.segments = {roomid: row["segments"] for roomid, row in store.initial_battles.items()}
        for roomid, row in store.initial_battles.items():
            history = store.initial_decisions.get(roomid, [])
            self.summaries[roomid] = {"sources": Counter(d["source"] for d in history),
                                      "notes": Counter(d["note"] for d in history if d["note"]),
                                      "times": [d["total_ms"] for d in history], "end": None, "rating": None,
                                      "errors": row["errors"], "refused": row["refused"],
                                      "fallbacks": sum(bool(d["fallback"]) or d["source"] == "fallback"
                                                       for d in history)}
        self.after_tasks: set[asyncio.Task] = set()
        self.snapshot_tasks: set[asyncio.Task] = set()
        self.search_unconfirmed_until = 0.0  # monotonic; see maybe_search
        self.base_count: int | None = None  # official games in the format at the run's (re)start
        self.counted_battles = 0  # rated battles finished since then
        self._snapshot_lock = asyncio.Lock()
        self.background: set[asyncio.Task] = set()
        self.room_tasks: set[asyncio.Task] = set()
        self.last_snapshot: dict | None = None
        self.battle_baselines = dict(store.initial_baselines)
        self.bad_battles = 0
        for row in sorted(store.initial_battles.values(), key=lambda b: b["ended"] or 0, reverse=True):
            if row["ended"] is None:
                continue
            fallbacks = self.summaries[row["room"]]["fallbacks"]
            if row["errors"] > 0 or fallbacks > 2:
                self.bad_battles += 1
            else:
                break
        self._connected_once = False
        self._search_lock = asyncio.Lock()
        self.last_replay_room: str | None = None

    def task(self, coroutine, group: set | None = None):
        tasks = group if group is not None else self.background
        task = asyncio.create_task(coroutine)
        tasks.add(task)
        def done(completed):
            tasks.discard(completed)
            if not completed.cancelled() and completed.exception() is not None:
                self.store.emit("task_error", {"error": type(completed.exception()).__name__})
                self.pause("ladder background task failed", 3)
        task.add_done_callback(done)
        return task

    async def send(self, text: str) -> None:
        if "|/pm " in text:
            return
        roomid, _, command = text.partition("|")
        if roomid.startswith("battle-"):
            if command == "/timer on":
                self.store.emit("timer_sent", {"time": time.time(), "text": text}, roomid)
            if not command.startswith("/choose"):
                self.store.emit("sent", {"time": time.time(), "text": text}, roomid)
        else:
            self.store.emit("sent", {"time": time.time(), "text": text})
        # The base send warning includes the command; never give it an assertion that it could log.
        ws = self.ws
        if ws is not None:
            async with self.send_lock:
                try:
                    await ws.send(text)
                except websockets.ConnectionClosed:
                    pass

    async def login(self, challstr: str) -> None:
        if self.stop_requested and not self.games:
            return
        delay = self.next_auth_at - time.monotonic()
        if delay > 0:
            await asyncio.sleep(delay)
        if self.config.get("local"):
            await super().login(challstr)
        else:
            try:
                url = self.config.get("login_url", "https://play.pokemonshowdown.com/api/login")
                assertion = await login_assertion(self.session, self.credentials.username, self.credentials.password,
                                                  challstr, url)
            except AuthError as error:
                await self.auth_failed(str(error))
                return
            self.store.add_secret(assertion)
            await self.send(f"|/trn {self.name},0,{assertion}")
        self.login_sent_at = time.monotonic()

    async def auth_failed(self, message: str) -> None:
        self.auth_failures += 1
        self.next_auth_at = time.monotonic() + (5, 30, 120)[min(self.auth_failures - 1, 2)]
        safe_message = self.store.clean(message)
        self.store.emit("auth_fail", {"count": self.auth_failures, "message": safe_message})
        log.warning("%s: authentication failed (%d/3): %s", self.name, self.auth_failures, safe_message)
        if self.auth_failures >= 3:
            self.pause("three consecutive authentication failures", 3)
        if self.ws:
            await self.ws.close()

    def on_message(self, message: str) -> None:
        if message.startswith(">"):
            roomid = message[1:].split("\n", 1)[0]
            self.capture_replay(roomid, message)
            super().on_message(message)
            return
        for line in message.splitlines():
            if line.startswith("|challstr|"):
                self.store.add_secret(line[len("|challstr|"):])
            self.store.emit("message", {"time": time.time(), "text": line})
            if line.startswith("|challstr|"):
                if self.login_task is None or self.login_task.done():
                    self.login_task = self.task(self.login(line[len("|challstr|"):]))
            elif line.startswith("|nametaken|"):
                self.task(self.auth_failed(line.split("|", 3)[-1]))
            elif line.startswith("|popup|"):
                self.store.emit("popup", {"text": line[len("|popup|"):]})
                self.capture_replay(None, line)
            elif line.startswith("|queryresponse|savereplay|"):
                self.capture_replay(None, line)
            else:
                super().on_message(line)

    def _on_updateuser(self, line: str) -> None:
        was_logged_in = self.connected
        parts = line.split("|")
        if len(parts) >= 4 and parts[3] == "1" and to_id(parts[2]) != self.userid:
            self.task(self.auth_failed("Server logged in a different account"))
            return
        if len(parts) >= 4 and parts[3] == "1" and to_id(parts[2]) == self.userid:
            self.logged_in.set()
            asyncio.get_running_loop().call_later(2.0, self._rejoin_games)
        if self.connected and not was_logged_in:
            self.auth_failures = 0
            self.login_sent_at = None
            self.reconcile_at = time.monotonic() + 2.0
            self.store.emit("login_ok", {"account": self.name})

    def _on_updatesearch(self, line: str) -> None:
        try:
            info = json.loads(line[len("|updatesearch|"):])
            self.searching = set(info.get("searching") or [])
        except (ValueError, AttributeError, TypeError):
            return
        if self.searching:
            self.search_unconfirmed_until = 0.0  # the server has our search
            if info.get("games") or self.rooms:
                # One battle at a time: a search still queued next to a running battle would start a second one.
                self.task(self.send("|/cancelsearch"))
                self.searching.clear()
        super()._on_updatesearch(line)
        self.store.emit("updatesearch", info)

    def _rejoin_games(self) -> None:
        if not self.connected:
            return
        for roomid in self.games - set(self.rooms) - self.done_rooms - self.rejoining:
            self.rejoining.add(roomid)
            self.store.emit("rejoin_requested", {"roomid": roomid})
            self.task(self.send(f"|/join {roomid}"))

    def _on_pm(self, line: str) -> None:
        parts = line.split("|", 4)
        if len(parts) == 5 and to_id(parts[3]) == self.userid and parts[4].startswith("/challenge "):
            sender = to_id(parts[2])
            if sender != self.userid:
                self.task(self.send(f"|/reject {sender}"))

    def _open_room(self, roomid: str, rejoined: bool = False):
        rejoined = rejoined or roomid in self.summaries or roomid in self.store.initial_battles
        room = super()._open_room(roomid, rejoined=rejoined)
        self.segments[roomid] = self.segments.get(roomid, 0) + 1
        room._ladder_segment = self.segments[roomid]
        self.room_tasks.add(room.task)
        room.task.add_done_callback(self.room_tasks.discard)
        now = time.time()
        wait = now - self.search_started if self.search_started is not None and not rejoined else None
        self.search_started = None
        self.search_unconfirmed_until = 0.0
        self.searching.clear()
        self.battle_baselines.setdefault(roomid, self.last_snapshot)
        self.summaries.setdefault(roomid, {"sources": Counter(), "notes": Counter(), "times": [], "end": None,
                                          "rating": None, "fallbacks": 0, "errors": 0, "refused": 0})
        self.store.emit("battle_open", {"time": now, "queue_wait_s": wait, "rejoined": rejoined}, roomid)
        self.store.emit("match", {"roomid": roomid, "queue_wait_s": wait})
        if rejoined:
            self.store.emit("rejoin", {}, roomid)
        return room

    def room_event(self, room, kind: str, data: dict) -> None:
        if kind == "decision":
            data = {**data, "battle_clock": getattr(room, "clock", None)}
        self.store.room_event(room, kind, data)
        summary = self.summaries.setdefault(room.room, {"sources": Counter(), "notes": Counter(), "times": [],
                                                        "end": None, "rating": None, "fallbacks": 0,
                                                        "errors": 0, "refused": 0})
        if kind == "decision":
            analysis = data.get("analysis") or {}
            source = data.get("source") or analysis.get("source", "default")
            summary["sources"][source] += 1
            summary["times"].append((data.get("timing") or {}).get("total_ms"))
            if analysis.get("note"):
                summary["notes"][analysis["note"]] += 1
            summary["fallbacks"] += int(bool(data.get("fallback")) or source == "fallback")
        elif kind == "rating" and data.get("userid") == self.userid:
            summary["rating"] = data
        elif kind == "refused":
            summary["refused"] += 1
        elif kind == "closed":
            summary["errors"] += (data.get("stats") or {}).get("errors", 0)
        elif kind == "end":
            summary["end"] = data
            if room.room not in self.ended_rooms:
                self.ended_rooms.add(room.room)
                self.finished_count += 1
            if self.config["save_replays"]:
                self.last_replay_room = room.room
                self.task(self.send(f"{room.room}|/savereplay"))
        elif kind == "unhealthy":
            reason = data.get("reason", "unhealthy room")
            stuck = any(word in reason.lower() for word in ("stuck", "hang", "worker", "past its deadline"))
            self.pause(reason, 4 if stuck else 3)
            if stuck:
                self.stuck_since = time.monotonic()

    def _room_ended(self, room) -> None:
        # An abandoned segment can finish after the new segment opens. It must not remove the new room.
        if self.rooms.get(room.room) is room:
            del self.rooms[room.room]
        summary = self.summaries.get(room.room, {})
        if room.ended and summary.get("end"):
            self.done_rooms.add(room.room)
            self.battles_played += 1
            self.task(self.after_battle(room), self.after_tasks)
        self.finished.put_nowait(room)

    async def after_battle(self, room) -> None:
        await self.send(f"{room.room}|/leave")
        summary = self.summaries[room.room]
        bad = summary["errors"] > 0 or room.stats.get("errors", 0) > 0 or summary["fallbacks"] > 2
        self.bad_battles = self.bad_battles + 1 if bad else 0
        if self.bad_battles >= 3:
            self.pause("three consecutive battles with errors or more than two fallbacks", 3)
        self.print_status(room, summary)
        if self.stuck_since is None:
            expected = None
            if room.rated and self.base_count is not None:
                # The official totals must reach the run's starting count plus every rated battle finished since.
                self.counted_battles += 1
                expected = self.base_count + self.counted_battles
            self.task(self.snapshot_after(room.room, expected), self.snapshot_tasks)

    async def snapshot_after(self, roomid: str, expected: int | None = None) -> None:
        """The official snapshot after a battle, in the background (matchmaking does not wait for it). One at a
        time, in battle order, so each battle's baseline already counts the battles before it."""
        async with self._snapshot_lock:
            if roomid not in self.store.initial_baselines and self.last_snapshot is not None:
                self.battle_baselines[roomid] = self.last_snapshot
            await self.capture_snapshot("after_battle", roomid, retry=True, expected=expected)

    def print_status(self, room, summary) -> None:
        end, rating, sources = summary["end"], summary["rating"] or {}, summary["sources"]
        outcome = "T" if (end.get("result") or {}).get("tie") else "W" if end.get("won") is True else \
                  "L" if end.get("won") is False else "?"
        foe = (room.players or {}).get("p2" if room.side == "p1" else "p1", {})
        times = summary["times"]
        total = sum(sources.values())
        log.info("game %d %s vs %s (%s) %d turns | elo %s -> %s | %d decisions: search %d, policy %d, "
                 "fallback %d, default %d | total p50 %.1f s p95 %.1f s max %.1f s | refused %d",
                 self.finished_count, outcome, foe.get("name", "?"), foe.get("rating", "?"), room.turn,
                 rating.get("old", "?"), rating.get("new", "?"), total, sources["search"], sources["policy"],
                 sources["fallback"], sources["default"], percentile(times, .5) / 1000,
                 percentile(times, .95) / 1000, percentile(times, 1) / 1000, summary["refused"])
        searching = self.config["bot"].get("search") is not False
        if searching and total and sources["search"] / total < .8:
            log.warning("search is not working: %d/%d search decisions; policy notes: %s", sources["search"], total,
                        "; ".join(f"{note} ({n})" for note, n in summary["notes"].most_common(4)) or "none reported")

    def capture_replay(self, roomid: str | None, message: str) -> None:
        url = re.search(r"(?:https?://)?replay\.pokemonshowdown\.com/([A-Za-z0-9_-]+)", message)
        replay_id = url.group(1) if url else None
        if "|queryresponse|savereplay|" in message:
            try:
                info = json.loads(message.split("|queryresponse|savereplay|", 1)[1].splitlines()[0])
                replay_id = info.get("id") or replay_id
            except (ValueError, AttributeError):
                pass
        if replay_id and re.fullmatch(r"[A-Za-z0-9_-]+", replay_id):
            roomid = roomid or ("battle-" + replay_id if "battle-" + replay_id in self.summaries else None)
            roomid = roomid or self.last_replay_room
            if roomid is None and len(self.ended_rooms) == 1:
                roomid = next(iter(self.ended_rooms))
            self.store.emit("replay", {"id": replay_id, "url": "https://replay.pokemonshowdown.com/" + replay_id},
                            roomid)

    async def capture_snapshot(self, stage: str, roomid: str | None = None, retry: bool = False,
                               expected: int | None = None) -> None:
        template = self.config.get("users_json")
        if not template or not self.session:
            return
        # A resumed battle can already be counted by the fresh run-start snapshot. Compare against its
        # pre-battle count, rather than demanding another game to appear in the official account totals.
        baseline = self.battle_baselines.get(roomid, self.last_snapshot) if retry else self.last_snapshot
        count = games(baseline) if baseline else None
        started = time.monotonic()
        for offset in ((0, 5, 20, 60, 180) if retry else (0,)):
            if offset:
                await asyncio.sleep(max(0, started + offset - time.monotonic()))
            try:
                async with self.session.get(template.format(userid=self.userid),
                                            timeout=aiohttp.ClientTimeout(total=15)) as response:
                    response.raise_for_status()
                    raw = await response.json(content_type=None)
                rating = raw.get("ratings", {}).get(self.config["format"])
                if not isinstance(rating, dict):
                    if stage == "start" and isinstance(raw, dict) and raw.get("userid"):
                        self.base_count = 0  # the account has not played this format yet
                    raise ValueError("No rating yet")
                if stage == "start":
                    self.base_count = games(rating)
                if expected is not None:
                    changed = games(rating) >= expected
                else:
                    changed = count is not None and games(rating) != count
                self.store.emit("ratings_snapshot", {"stage": stage, "battle_room": roomid, "rating": rating,
                                                      "raw": raw, "pending": bool(retry and not changed)})
                self.last_snapshot = rating
                if not retry or changed:
                    return
            except (aiohttp.ClientError, TimeoutError, ValueError, AttributeError, TypeError):
                self.store.emit("ratings_snapshot_error", {"stage": stage, "battle_room": roomid, "offset_s": offset})
        if retry:
            self.store.emit("ratings_pending", {"battle_room": roomid, "baseline": baseline})

    def pause(self, reason: str, code: int = 3) -> None:
        if not self.exit_code or code == 4:
            self.exit_code = code
            self.store.emit("pause", {"reason": reason, "exit_code": code})
            log.warning("pausing matchmaking: %s", reason)
        self.request_stop(reason)

    def request_stop(self, reason: str = "signal", force: bool = False) -> None:
        if force:
            self.force_stop = True
            log.warning("exiting now; any unfinished battle keeps running on the server")
        if not self.stop_requested:
            self.stop_requested = True
            self.store.emit("stop", {"reason": reason, "force": force})
            log.info("stopping new matchmaking; waiting for the current battle and rating")

    def check_limits(self) -> None:
        limits = self.config["stop"]
        if limits.get("minutes") is not None and time.time() - self.store.manifest["started"] >= limits["minutes"] * 60:
            self.request_stop("minutes limit")
        if limits.get("games") is not None and self.finished_count >= limits["games"]:
            self.request_stop("games limit")

    async def maybe_search(self) -> None:
        async with self._search_lock:
            self.check_limits()
            if (self.stop_requested or self.store.failed.is_set() or not self.connected or self.games or self.rooms
                    or self.after_tasks or self.searching or time.monotonic() < self.reconcile_at
                    or time.monotonic() < self.search_unconfirmed_until):
                return
            self.searching.add(self.config["format"])
            # Until the server lists the search (or a battle starts), a "not searching" update may predate our
            # /search: searching again then queues a second search, and the server starts two battles.
            self.search_unconfirmed_until = time.monotonic() + SEARCH_CONFIRM_S
            self.search_started = time.time()
            self.store.emit("search", {"time": self.search_started, "format": self.config["format"]})
            await self.send("|/utm null")
            await self.send(f"|/search {self.config['format']}")

    async def run(self) -> None:
        await self.worker.ready.wait()
        backoff = 1.0
        while not self.stopping:
            try:
                async with websockets.connect(self.url, max_size=None, ping_interval=20, ping_timeout=60,
                                              open_timeout=10) as ws:
                    self.ws = ws
                    self.reconcile_at = float("inf")
                    self.searching.clear()
                    self.rejoining.clear()
                    self.store.emit("reconnect" if self._connected_once else "connect", {"server": self.url})
                    self._connected_once = True
                    backoff = 1.0
                    async for message in ws:
                        self.on_message(message.decode("utf-8", "replace") if isinstance(message, bytes) else message)
            except asyncio.CancelledError:
                raise
            except (OSError, TimeoutError, websockets.WebSocketException):
                if not self.stopping:
                    self.store.emit("connection_error", {"server": self.url})
            finally:
                # Preserve the authoritative games set and unfinished records across socket drops.
                self.games.update(r.room for r in self.rooms.values() if not r.ended)
                self.ws = None
                self.logged_in.clear()
                if self.login_task and not self.login_task.done():
                    self.login_task.cancel()
                self.login_sent_at = None
                self._abandon_rooms()
                if not self.stopping:
                    self.store.emit("disconnect", {"pending_rooms": sorted(self.games - self.done_rooms)})
            if not self.stopping:
                await asyncio.sleep(max(backoff, self.next_auth_at - time.monotonic()))
                backoff = min(backoff * 2, 30)

    async def drive(self) -> int:
        """Monitor limits and health independently of socket and room tasks."""
        await self.capture_snapshot("start")
        connection = self.task(self.run())
        proc = psutil.Process()
        proc.cpu_percent()
        psutil.cpu_percent()
        next_resources = time.monotonic()
        recorder_reported = False
        try:
            while True:
                now = time.monotonic()
                self.check_limits()
                if self.store.failed.is_set() and not recorder_reported:
                    self.store.emit("recorder_error", {"reason": self.store.failure})
                    self.pause("recorder write failure", 3)
                    recorder_reported = True
                if self.bad_battles >= 3 and not self.exit_code:
                    self.pause("three consecutive battles with errors or more than two fallbacks", 3)
                if self.login_sent_at and not self.connected and now - self.login_sent_at >= 20:
                    self.login_sent_at = None
                    await self.auth_failed("Server did not confirm login within 20 seconds")
                if now >= next_resources:
                    try:
                        self.store.emit("resources", {"cpu": psutil.cpu_percent(), "proc_cpu": proc.cpu_percent(),
                                                       "rss_mb": proc.memory_info().rss / 1024 ** 2,
                                                       "mem_percent": psutil.virtual_memory().percent,
                                                       "worker_queue": self.worker.executor._work_queue.qsize(),
                                                       "rooms": len(self.rooms)})
                    except psutil.Error:
                        self.store.emit("resources_error")
                    next_resources = now + 15
                if self.force_stop:
                    return 130
                if self.stuck_since is not None and (not any(not r.ended for r in self.rooms.values())
                                                     or now - self.stuck_since >= 120):
                    return 4
                if self.stop_requested:
                    if self.connected and self.searching:
                        await self.send("|/cancelsearch")
                        self.searching.clear()
                    # Finished games can linger in updatesearch; unresolved games require reconnect/rejoin.
                    live_games = self.games - self.done_rooms
                    if not self.rooms and not self.after_tasks and (not live_games or self.auth_failures >= 3):
                        if self.snapshot_tasks:  # let the last battles' official ratings arrive (at most ~3 min)
                            await asyncio.wait(set(self.snapshot_tasks), timeout=200)
                        await self.capture_snapshot("end")
                        return self.exit_code
                else:
                    await self.maybe_search()
                await asyncio.sleep(.1)
        finally:
            self.stopping = True
            connection.cancel()
            tasks = list(self.background | self.after_tasks | self.snapshot_tasks)
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)
            if self.force_stop or self.stuck_since is not None:
                for room in self.rooms.values():
                    if room.task:
                        room.task.cancel()
                self.rooms.clear()
            else:
                await self.stop()
            if self.room_tasks:
                if self.force_stop or self.stuck_since is not None:
                    for task in list(self.room_tasks):
                        task.cancel()
                await asyncio.wait(list(self.room_tasks), timeout=.1 if self.force_stop or self.stuck_since else 10)


async def main(args: argparse.Namespace) -> int:
    worker, store, client = None, None, None
    old_signals = {}
    try:
        runs_dir, db = absolute_path(args.runs_dir), absolute_path(args.db)
        previous = None
        if args.resume:
            previous = json.loads((runs_dir / safe_id(args.resume) / "manifest.json").read_text(encoding="utf-8"))
        path = Path(args.config) if args.config else HERE / ("ladder-local.json" if args.local else "ladder.json")
        if not path.exists() and args.config:
            path = HERE.parent / args.config
        config = load_config(path, local=args.local, base=previous["config"] if previous and not args.config else None)
        for key in ("minutes", "games"):
            value = getattr(args, key)
            if value is not None:
                config["stop"][key] = value
        if args.credentials:
            config["credentials"] = str(Path(args.credentials).expanduser().resolve())
        config = load_config(None, base=config, local=args.local)  # validate CLI overrides as well as the file
        if config.get("numbrion_path"):
            sys.path.insert(0, config["numbrion_path"])
        credentials = None if config.get("local") else load_credentials(config.get("credentials"))
        if credentials and to_id(credentials.username) != to_id(config["account"]):
            raise ConfigError("Credentials username must match config.account")
        run_id = args.resume or args.run_id or make_run_id(config["account"])
        if not args.resume and (runs_dir / safe_id(run_id) / "manifest.json").exists():
            raise ConfigError("Run id already exists; use --resume or a new --run-id")
        manifest = build_manifest(config, run_id, previous["started"] if previous else None)
        manifest["runs_dir"] = str(runs_dir)
        differences = validate_resume(previous, manifest, args.force) if previous else []
        if previous:
            # Preserve the original identity. Forced changes remain explicitly visible in the manifest and events.
            current = manifest
            manifest = {**(current if differences else previous), "resumes": [*previous.get("resumes", []),
                        {"time": time.time(), "differences": differences, "identity": current}]}
            if differences:
                manifest["original_identity"] = previous.get("original_identity", previous)
                log.warning("forced resume changes frozen identity: %s", ", ".join(differences))
        worker = BotWorker(config["bot"])
        await worker.load()
        manifest["bot"]["description"] = worker.describe()
        preflight = {"ok": True, "summary": "Skipped by --no-preflight", "skipped": True}
        reuse = previous is not None and not differences and (previous.get("preflight") or {}).get("ok")
        if reuse:
            # The same frozen bot passed it at the run's start (hashes checked above); a battle in progress may be
            # waiting for us to reconnect (Showdown forfeits a player away for about a minute with the timer on).
            preflight = {**previous["preflight"], "reused": True}
        elif not args.no_preflight and not worker.failed:
            try:
                from play.preflight import run_preflight
                log.info("preflight: one short in-process battle with the run's time rules")
                preflight = await worker.run(run_preflight, worker.bot, DecisionTiming.from_dict(config["timing"]))
                log.info("preflight %s: %s", "passed" if preflight.get("ok") else "FAILED", preflight.get("summary"))
                if not isinstance(preflight, dict) or not isinstance(preflight.get("ok"), bool):
                    raise ConfigError("Preflight returned an invalid result")
            except ModuleNotFoundError as error:
                if error.name == "play.preflight":
                    log.warning("play.preflight is missing; skipping preflight")
                    preflight = {"ok": True, "summary": "play.preflight missing", "skipped": True}
                else:
                    preflight = {"ok": False, "summary": "Preflight dependency is missing"}
            except Exception as error:
                preflight = {"ok": False, "summary": f"Preflight failed ({type(error).__name__})"}
        manifest["preflight"] = preflight
        store = RunStore(runs_dir, db, manifest, (credentials.password,) if credentials else ())
        store.emit("resume" if previous else "run_start", {"run_id": run_id, "forced_changes": differences})
        if worker.failed or not preflight.get("ok"):
            store.emit("preflight", {"result": preflight, "load_failed": bool(worker.failed)})
            log.error("bot startup failed: %s", "bot load failed" if worker.failed else
                      store.clean(preflight.get("summary", "preflight failed")))
            await asyncio.wrap_future(store.finish("config_error"))
            return 2
        async with aiohttp.ClientSession() as session:
            client = LadderClient(worker, config, store, credentials, session)
            signals_seen = 0
            def stopped(signum, frame):
                nonlocal signals_seen
                signals_seen += 1
                client.request_stop(signal.Signals(signum).name, force=signals_seen >= 2)
                if signals_seen >= 2:
                    sys.stdout.flush()
                    sys.stderr.flush()
                    KeepAwake().__exit__()
                    os._exit(130)
            for sig in {signal.SIGINT, getattr(signal, "SIGBREAK", signal.SIGINT), signal.SIGTERM}:
                old_signals[sig] = signal.signal(sig, stopped)
            log.info("run %s | %s | %s", run_id, config["account"], config["format"])
            code = await client.drive()
        state = {0: "complete", 3: "paused", 4: "worker_stuck", 130: "interrupted"}.get(code, "crashed")
        if code == 0 and store.failed.is_set():
            state = "paused"
        await asyncio.wrap_future(store.finish(state))
        return 3 if store.failed.is_set() and code == 0 else code
    except Exception as error:
        # Config/credentials errors may include user data; emit the known-safe class messages only.
        log.error("%s", str(error) if isinstance(error, (ConfigError, AuthError)) else
                  f"Configuration or recorder setup failed ({type(error).__name__})")
        if isinstance(error, RecorderError):
            return 3
        if client is not None:
            if store:
                store.emit("crash", {"error": type(error).__name__})
                await asyncio.wrap_future(store.finish("crashed"))
            return 1
        return 2
    finally:
        for sig, handler in old_signals.items():
            signal.signal(sig, handler)
        if worker:
            worker.shutdown()
        if store:
            store.close()


def parse_args(argv=None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--config")
    parser.add_argument("--local", action="store_true")
    parser.add_argument("--minutes", type=float)
    parser.add_argument("--games", type=int)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--run-id")
    group.add_argument("--resume")
    parser.add_argument("--force", action="store_true",
                        help="resume despite changed frozen identity; recorded explicitly")
    parser.add_argument("--runs-dir", default="scratch/ladder/runs")
    parser.add_argument("--db", default="scratch/ladder/ladder.sqlite")
    parser.add_argument("--credentials")
    parser.add_argument("--no-preflight", action="store_true")
    parser.add_argument("-v", "--verbose", action="store_true")
    return parser.parse_args(argv)


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):  # user names can hold characters a Windows console cannot show
        stream.reconfigure(errors="replace")
    arguments = parse_args()
    logging.basicConfig(level=logging.DEBUG if arguments.verbose else logging.INFO,
                        format="%(asctime)s %(levelname)s %(message)s", datefmt="%H:%M:%S")
    # websockets DEBUG logs contain unredacted frames, including assertions.
    logging.getLogger("websockets").setLevel(logging.WARNING)
    logging.getLogger("aiohttp").setLevel(logging.WARNING)
    with KeepAwake():
        status = asyncio.run(main(arguments))
    if status in {4, 130}:
        # Python waits for ThreadPoolExecutor threads at exit; a stuck native call cannot be joined.
        os._exit(status)
    sys.exit(status)
