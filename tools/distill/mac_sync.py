#!/usr/bin/env python3
"""Relay atomic label shards and numbered teachers through the laptop; --local never invokes SSH."""
from __future__ import annotations

import argparse
import base64
import fcntl
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

MAC_REPO = Path(os.environ.get("DISTILL_MAC_REPO", "/Users/aminaliu/.t3/projects/numbrion-mac"))
WINDOWS_REPO = os.environ.get("DISTILL_WINDOWS_REPO", "C:/Users/aminu/.t3/projects/numbrion")
CONFIG = dict(
    labels=Path(os.environ.get("DISTILL_LABELS", MAC_REPO / "scratch/distill-loop/labels")),
    teacher=Path(os.environ.get("DISTILL_TEACHER", MAC_REPO / "scratch/distill-loop/teacher")),
    state=Path(os.environ.get("DISTILL_SYNC_STATE", MAC_REPO / "scratch/distill-loop/sync")),
    laptop=os.environ.get("DISTILL_LAPTOP", "amin@100.105.66.101"),
    port=int(os.environ.get("DISTILL_LAPTOP_PORT", "91")),
    key=os.environ.get("DISTILL_HOP_KEY", "/home/amin/.ssh/id_ed25519"),
    relay=os.environ.get("DISTILL_RELAY", "/home/amin/distill-relay"),
    windows=os.environ.get("DISTILL_WINDOWS", "amin@100.96.185.127"),
    windows_labels=os.environ.get("DISTILL_WINDOWS_LABELS", WINDOWS_REPO + "/scratch/distill-loop/labels"),
    windows_snapshots=os.environ.get("DISTILL_WINDOWS_SNAPSHOTS", WINDOWS_REPO + "/scratch/distill-loop/train/snapshots"),
    control=os.environ.get("DISTILL_CONTROL_PATH", "/tmp/numbrion-distill-%C"),
    local=os.environ.get("DISTILL_LOCAL", "0") == "1",
    local_laptop=Path(os.environ.get("DISTILL_LOCAL_LAPTOP", "/tmp/distill-local/laptop")),
    local_windows=Path(os.environ.get("DISTILL_LOCAL_WINDOWS", "/tmp/distill-local/windows")),
    shard_seconds=float(os.environ.get("DISTILL_SHARD_SECONDS", "30")),
    teacher_seconds=float(os.environ.get("DISTILL_TEACHER_SECONDS", "300")),
    batch=int(os.environ.get("DISTILL_SYNC_BATCH", "64")),
)


@dataclass
class Config:
    labels: Path
    teacher: Path
    state: Path
    laptop: str
    port: int
    key: str
    relay: str
    windows: str
    windows_labels: str
    windows_snapshots: str
    control: str
    local: bool
    local_laptop: Path
    local_windows: Path
    shard_seconds: float
    teacher_seconds: float
    batch: int


def atomic_json(path: Path, value):
    with path.with_suffix(path.suffix + ".tmp").open("w", encoding="utf-8") as stream:
        json.dump(value, stream, sort_keys=True)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(path.with_suffix(path.suffix + ".tmp"), path)


def newest_snapshot(paths):
    numbered = [(int(m[1]), p) for p in paths if (m := re.fullmatch(r"m(\d+)\.pt", Path(p).name))]
    return max(numbered, key=lambda item: (item[0], str(item[1])), default=(None, None))[1]


def teacher_key(path):
    match = re.fullmatch(r"m(\d+)\.pt", path.name)
    return (1, int(match[1]), path.stat().st_mtime_ns) if match else (0, path.stat().st_mtime_ns, path.name)


def ps_quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def powershell(script):
    script = '$ErrorActionPreference="Stop"; $ProgressPreference="SilentlyContinue"; ' + script
    encoded = base64.b64encode(script.encode("utf-16le")).decode("ascii")
    return "powershell -NoProfile -EncodedCommand " + encoded


def publish_script(directory, names):
    names = ",".join(ps_quote(n) for n in names)
    return (f"$dir={ps_quote(directory)}; foreach ($name in @({names})) {{ "
            "$part=Join-Path $dir ($name+'.part'); $final=Join-Path $dir $name; "
            "$used=Join-Path (Join-Path $dir 'used') $name; "
            "$held=Join-Path (Join-Path $dir 'heldout') $name; "
            "if ((Test-Path -LiteralPath $final) -or (Test-Path -LiteralPath $used) -or "
            "(Test-Path -LiteralPath $held)) { Remove-Item -LiteralPath $part -ErrorAction SilentlyContinue } "
            "else { Move-Item -LiteralPath $part -Destination $final -ErrorAction Stop } }")


class Remote:
    def __init__(self, config):
        self.c = config
        self.options = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=15",
                        "-o", "ServerAliveCountMax=3", "-o", "ControlMaster=auto", "-o", "ControlPersist=600",
                        "-o", "ControlPath=" + config.control]

    def laptop(self, command, stdout=subprocess.PIPE):
        return subprocess.run(["ssh", "-p", str(self.c.port), *self.options, self.c.laptop, command],
                              stdout=stdout, stderr=subprocess.PIPE, check=True, timeout=180)

    def windows_command(self, script):
        return shlex.join(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-i", self.c.key,
                           self.c.windows, powershell(script)])

    def publish(self, paths):
        c, names = self.c, [p.name for p in paths]
        batch = hashlib.sha256("\n".join(names).encode()).hexdigest()[:16]
        relay = c.relay.rstrip("/") + "/" + batch
        with tempfile.TemporaryDirectory(dir=c.state, prefix="batch-") as staging:
            parts = []
            for path in paths:
                part = Path(staging) / (path.name + ".part")
                shutil.copyfile(path, part)
                parts.append(str(part))
            setup = f"New-Item -ItemType Directory -Force -Path {ps_quote(c.windows_labels)} | Out-Null"
            self.laptop("mkdir -p " + shlex.quote(relay) + " && " + self.windows_command(setup))
            subprocess.run(["scp", "-q", "-P", str(c.port), *self.options, *parts, c.laptop + ":" + relay + "/"],
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, timeout=180)
            copy = shlex.join(["scp", "-q", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-i", c.key,
                               *[relay + "/" + n + ".part" for n in names],
                               c.windows + ":" + c.windows_labels.rstrip("/") + "/"])
            self.laptop(copy + " && " + self.windows_command(publish_script(c.windows_labels, names)) +
                        " && rm -rf " + shlex.quote(relay))

    def newest(self):
        script = (f"$dir={ps_quote(self.c.windows_snapshots)}; if (Test-Path -LiteralPath $dir) {{ "
                  "$f=Get-ChildItem -LiteralPath $dir -File | Where-Object { $_.Name -match '^m[0-9]+\\.pt$' } | "
                  "Sort-Object { [long]($_.BaseName.Substring(1)) } -Descending | Select-Object -First 1; "
                  "if ($f) { Write-Output $f.Name } }")
        result = self.laptop(self.windows_command(script))
        names = result.stdout.decode("utf-8-sig").strip().splitlines()
        return newest_snapshot(names)

    def fetch(self, name, destination):
        c = self.c
        temporary = c.relay.rstrip("/") + "/teacher-" + name
        copy = shlex.join(["scp", "-q", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-i", c.key,
                           c.windows + ":" + c.windows_snapshots.rstrip("/") + "/" + name, temporary])
        command = ("mkdir -p " + shlex.quote(c.relay) + " && " + copy + " && cat " + shlex.quote(temporary) +
                   " && rm -f " + shlex.quote(temporary))
        with destination.open("wb") as stream:
            self.laptop(command, stdout=stream)
            stream.flush()
            os.fsync(stream.fileno())


class Local:
    """Exercise the same copy/rename boundary and retry semantics using local laptop and Windows directories."""

    def __init__(self, config):
        self.c = config
        self.labels, self.snapshots = config.local_windows / "labels", config.local_windows / "train/snapshots"
        config.local_laptop.mkdir(parents=True, exist_ok=True)
        self.labels.mkdir(parents=True, exist_ok=True)

    def publish(self, paths):
        for path in paths:
            relay = self.c.local_laptop / (path.name + ".part")
            shutil.copyfile(path, relay)
            shutil.copyfile(relay, self.labels / relay.name)
        if (self.c.local_windows / "FAIL_TRANSFER").exists():
            raise OSError("LOCAL injected transfer failure before rename")
        for path in paths:
            part = self.labels / (path.name + ".part")
            if any((directory / path.name).exists() for directory in
                   (self.labels, self.labels / "used", self.labels / "heldout")):
                part.unlink()
            else:
                os.replace(part, self.labels / path.name)
            (self.c.local_laptop / part.name).unlink()

    def newest(self):
        return newest_snapshot(p.name for p in self.snapshots.glob("*.pt") if p.is_file())

    def fetch(self, name, destination):
        relay = self.c.local_laptop / (name + ".part")
        shutil.copyfile(self.snapshots / name, relay)
        with relay.open("rb") as source, destination.open("wb") as stream:
            shutil.copyfileobj(source, stream)
            stream.flush()
            os.fsync(stream.fileno())
        relay.unlink()


class Sync:
    def __init__(self, config, remote=None):
        self.c = config
        for directory in (config.labels, config.teacher, config.state):
            directory.mkdir(parents=True, exist_ok=True)
        self.remote = remote or (Local(config) if config.local else Remote(config))
        self.ledger_path = config.state / "sent.json"
        self.sent = set(json.loads(self.ledger_path.read_text())) if self.ledger_path.exists() else set()
        self.last_error, self.failures = "", {"shards": 0, "teacher": 0}
        self.due = {"shards": 0.0, "teacher": 0.0}

    def backlog(self):
        return sorted(p for p in self.c.labels.glob("labels-*.npz") if p.is_file() and p.name not in self.sent)

    def send(self):
        paths = self.backlog()[:self.c.batch]
        if paths:
            self.remote.publish(paths)
            sent = self.sent | {p.name for p in paths}
            atomic_json(self.ledger_path, sorted(sent))
            self.sent = sent

    def pull(self):
        name = self.remote.newest()
        current = newest_snapshot(p for p in self.c.teacher.glob("*.pt") if p.is_file())
        if name and (current is None or int(name[1:-3]) > int(current.name[1:-3])):
            final = self.c.teacher / name
            part = self.c.teacher / (name + ".part")
            self.remote.fetch(name, part)
            if part.stat().st_size == 0:
                raise OSError(f"empty teacher {name}")
            os.replace(part, final)
            print(f"Teacher pulled: {name}", flush=True)
        teachers = [p for p in self.c.teacher.glob("*.pt") if p.is_file()]
        teachers.sort(key=teacher_key, reverse=True)
        for old in teachers[3:]:
            old.unlink()

    def round(self, force=False):
        now, errors = time.monotonic(), []
        for kind, operation, interval in (("shards", self.send, self.c.shard_seconds),
                                          ("teacher", self.pull, self.c.teacher_seconds)):
            if not force and now < self.due[kind]:
                continue
            try:
                operation()
                self.failures[kind] = 0
                self.due[kind] = now + interval
            except (OSError, subprocess.SubprocessError, ValueError) as error:
                self.failures[kind] += 1
                self.due[kind] = now + min(300, 5 * 2 ** min(self.failures[kind] - 1, 6))
                detail = str(error)
                if isinstance(error, subprocess.CalledProcessError) and error.stderr:
                    detail += " " + error.stderr.decode(errors="replace").strip()
                errors.append(kind + ": " + detail.replace("\n", " "))
        if errors:
            self.last_error = "; ".join(errors)
        elif not any(self.failures.values()):
            self.last_error = ""
        current = max((p for p in self.c.teacher.glob("*.pt") if p.is_file()), key=teacher_key, default=None)
        status = dict(sent=len(self.sent), backlog=len(self.backlog()),
                      teacher=current.name if current else "none", last_error=self.last_error,
                      updated=time.time())
        atomic_json(self.c.state / "status.json", status)
        print(f"Sync: shards_sent={status['sent']} backlog={status['backlog']} "
              f"teacher={status['teacher']} last_error={self.last_error or 'none'}", flush=True)
        return not any(self.failures.values())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--local", action="store_true", default=CONFIG["local"])
    parser.add_argument("--once", action="store_true", help="one send/pull round and exit")
    for key in ("labels", "teacher", "state", "local_laptop", "local_windows"):
        parser.add_argument("--" + key.replace("_", "-"), type=Path, default=CONFIG[key])
    args = parser.parse_args()
    config = Config(**{**CONFIG, **{k: v for k, v in vars(args).items() if k != "once"}})
    if config.batch < 1 or min(config.shard_seconds, config.teacher_seconds) <= 0:
        parser.error("batch and poll intervals must be positive")
    config.state.mkdir(parents=True, exist_ok=True)
    with (config.state / "sync.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            raise SystemExit("another sync process holds " + str(config.state / "sync.lock"))
        run(config, args.once)


def run(config, once=False):
    sync, stopping = Sync(config), False

    def stop(signum, frame):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    if once:
        raise SystemExit(0 if sync.round(force=True) else 1)
    while not stopping:
        sync.round()
        wake = min(sync.due.values())
        while not stopping and time.monotonic() < wake:
            time.sleep(min(1, max(0, wake - time.monotonic())))
    sync.round(force=True)


if __name__ == "__main__":
    main()
