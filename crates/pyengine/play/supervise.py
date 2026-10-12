"""Restart ladder crashes and stuck workers, preserving the run id and frozen identity."""
from __future__ import annotations

import argparse
import os
import signal
import subprocess
import sys
import time
from collections import deque
from pathlib import Path

HERE = Path(__file__).resolve().parent
if sys.path and Path(sys.path[0] or ".").resolve() == HERE:
    sys.path[0] = str(HERE.parent)
elif str(HERE.parent) not in sys.path:
    sys.path.insert(0, str(HERE.parent))

from play.ladder import KeepAwake, absolute_path, load_config, make_run_id, parse_args  # noqa: E402
from play.runstore import canonical, safe_id  # noqa: E402


def child_args(arguments: list[str], run_id: str, resume: bool) -> list[str]:
    """Replace, rather than duplicate, an existing run-id or resume option."""
    result = []
    skip = False
    for arg in arguments:
        if skip:
            skip = False
        elif arg in {"--run-id", "--resume"}:
            skip = True
        elif not arg.startswith(("--run-id=", "--resume=")):
            result.append(arg)
    return [*result, "--resume" if resume else "--run-id", run_id]


def record(path: Path, kind: str, **data) -> None:
    # The child has exited before exit/start records are appended, so there are no concurrent file writers.
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(canonical({"t": time.time(), "kind": kind, "room": None, **data}) + "\n")
        handle.flush()


def run(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--max-restarts", type=int, default=5)
    parser.add_argument("--window-min", type=float, default=30)
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    arguments = args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
    if not arguments:
        parser.error("pass ladder arguments after --")
    if args.max_restarts < 0 or args.window_min <= 0:
        parser.error("restart limit must be nonnegative and window must be positive")
    ladder_args = parse_args(arguments)
    runs_dir = absolute_path(ladder_args.runs_dir)
    try:
        if ladder_args.resume:
            run_id = safe_id(ladder_args.resume)
        else:
            config_path = Path(ladder_args.config) if ladder_args.config else HERE / (
                "ladder-local.json" if ladder_args.local else "ladder.json")
            if not config_path.exists() and ladder_args.config:
                config_path = HERE.parent / ladder_args.config
            config = load_config(config_path, ladder_args.local)
            run_id = safe_id(ladder_args.run_id or make_run_id(config["account"]))
        event_path = runs_dir / run_id / "events.jsonl"
    except (OSError, ValueError, KeyError, TypeError):
        print("Supervisor configuration failed", file=sys.stderr)
        return 2
    stopped, child = False, None
    old_handlers = {}

    def interrupted(signum, frame):
        nonlocal stopped
        stopped = True
        if child and child.poll() is None:
            try:
                child.send_signal(signal.CTRL_BREAK_EVENT if os.name == "nt" else signal.SIGINT)
            except (OSError, ProcessLookupError):
                pass

    for sig in {signal.SIGINT, getattr(signal, "SIGBREAK", signal.SIGINT), signal.SIGTERM}:
        old_handlers[sig] = signal.signal(sig, interrupted)
    restarts = deque()
    attempt = 0
    try:
        with KeepAwake():
            while not stopped:
                # A crash before a manifest exists cannot be resumed; repeat initialization with the same id.
                resume = bool(ladder_args.resume or (runs_dir / run_id / "manifest.json").exists())
                record(event_path, "supervisor_start", attempt=attempt, resume=resume)
                command = [sys.executable, str(HERE / "ladder.py"), *child_args(arguments, run_id, resume)]
                options = {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == "nt" else {}
                child = subprocess.Popen(command, **options)
                while child.poll() is None:
                    time.sleep(.2)
                code = child.returncode
                record(event_path, "supervisor_exit", attempt=attempt, exit_code=code, interrupted=stopped)
                if stopped or code in {0, 2, 3}:
                    return code
                now = time.monotonic()
                while restarts and now - restarts[0] > args.window_min * 60:
                    restarts.popleft()
                if len(restarts) >= args.max_restarts:
                    record(event_path, "supervisor_limit", restarts=len(restarts), window_min=args.window_min)
                    print("Supervisor restart limit reached", file=sys.stderr)
                    return code or 1
                restarts.append(now)
                delay = (10, 30, 60, 120)[min(attempt, 3)]
                record(event_path, "supervisor_backoff", seconds=delay)
                until = now + delay
                while not stopped and time.monotonic() < until:
                    time.sleep(.2)
                attempt += 1
            return 130
    except OSError:
        print("Supervisor could not start or record the child", file=sys.stderr)
        return 2
    finally:
        for sig, handler in old_handlers.items():
            signal.signal(sig, handler)


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):  # user names can hold characters a Windows console cannot show
        stream.reconfigure(errors="replace")
    sys.exit(run())
