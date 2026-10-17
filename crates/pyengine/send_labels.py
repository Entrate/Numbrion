"""Send finished label shards to another machine's label directory over SSH (scp, then an atomic mv).

    .venv/Scripts/python.exe crates/pyengine/send_labels.py scratch/win-labels/NAME \\
        --to aminaliu@100.98.169.85:/Users/aminaliu/.t3/projects/numbrion-mac/scratch/gen3/labels-win

Runs beside shallow_labels.py (or search_labels.py) on the same ``--out`` directory. Every ``--interval`` seconds it
sends the finished ``labels-*.npz`` shards, oldest first and ``--batch`` per call (``.npz.tmp`` files are still
being written and are skipped). scp puts them into ``<remote>/.incoming/``, and one ssh ``mv`` then renames them into
``<remote>/``. A reader of the remote directory (distill_train --follow globs only ``labels-*.npz`` there and in
``used/`` / ``heldout/``) never sees a partial file. Each sent shard is then moved into ``<dir>/sent/``
(``--delete-sent``: deleted). A failed call is retried on the next pass: the same names overwrite the half-sent
copies in ``.incoming/``. Only a lost reply after a successful mv can resend a shard. The directory's
``config.json`` goes once as ``<dir name>-config.json``. One sender per directory (``.send.lock``). Key login
(``ssh -o BatchMode=yes``) must work.

Stops after a complete pass once the label maker, seen running at least once, has finished (its ``.labels.lock`` is
free again, e.g. after ``<dir>/STOP``), on ``<dir>/STOP_SEND`` after a complete pass, or at Ctrl+C. It can start
before the maker. ``--once``: one pass (e.g. for a directory whose maker has already finished).
"""
from __future__ import annotations

import argparse
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from search_labels import output_lock  # noqa: E402

SSH_OPTIONS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-o", "ServerAliveInterval=15"]


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("dir", type=Path, help="the label maker's --out directory")
    parser.add_argument("--to", required=True, help="USER@HOST:/absolute/remote/label/directory")
    parser.add_argument("--interval", type=float, default=60.0, help="seconds between passes")
    parser.add_argument("--batch", type=int, default=64, help="shards per scp call")
    parser.add_argument("--timeout", type=float, default=600.0, help="seconds per scp / ssh call")
    parser.add_argument("--delete-sent", action="store_true", help="delete sent shards instead of moving them to sent/")
    parser.add_argument("--once", action="store_true", help="one pass, then exit")
    args = parser.parse_args(argv)
    host, _, remote = args.to.partition(":")
    if not host or not remote.startswith("/"):
        parser.error("--to must be USER@HOST:/absolute/path")
    args.host, args.remote = host, remote.rstrip("/")
    if args.batch < 1 or args.interval <= 0 or args.timeout <= 0:
        parser.error("--batch, --interval and --timeout must be positive")
    return args


def maker_running(directory: Path) -> bool:
    """True while a label maker holds the directory's lock (search_labels.output_lock)."""
    if not (directory / ".labels.lock").exists():
        return False
    try:
        with output_lock(directory):
            return False
    except RuntimeError:
        return True


def run(args, command: list[str], what: str) -> bool:
    try:
        # Relative names from the label directory: a Windows drive letter would read as an scp host.
        result = subprocess.run(command, cwd=args.dir, capture_output=True, text=True, timeout=args.timeout)
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"Send failed ({what}): {error}", flush=True)
        return False
    if result.returncode:
        print(f"Send failed ({what}, exit {result.returncode}): {(result.stderr or result.stdout).strip()[:500]}",
              flush=True)
        return False
    return True


def remote(args, script: str, what: str) -> bool:
    return run(args, ["ssh", *SSH_OPTIONS, args.host, script], what)


def upload(args, names: list[str]) -> bool:
    """scp into <remote>/.incoming, then one mv into <remote> (rename on one file system: atomic per file)."""
    incoming = f"{args.remote}/.incoming"
    if not run(args, ["scp", "-q", *SSH_OPTIONS, *names, f"{args.host}:{shlex.quote(incoming)}/"],
               f"scp {len(names)} files"):
        return False
    moves = " ".join(shlex.quote(name) for name in names)
    return remote(args, f"cd {shlex.quote(incoming)} && mv -f {moves} ..", f"mv {len(names)} files")


def archive(args, path: Path, sent_dir: Path) -> None:
    if args.delete_sent:
        path.unlink(missing_ok=True)
    else:
        sent_dir.mkdir(exist_ok=True)
        path.replace(sent_dir / path.name)


def send_pass(args, totals: dict) -> bool:
    """Send every finished shard; False if a call failed (the rest waits for the next pass)."""
    sent_dir = args.dir / "sent"
    config = args.dir / "config.json"
    config_name = f"{args.dir.name}-config.json"
    if config.exists() and not (sent_dir / config_name).exists():
        copy = args.dir / config_name
        shutil.copyfile(config, copy)
        if not upload(args, [config_name]):
            return False
        sent_dir.mkdir(exist_ok=True)
        copy.replace(sent_dir / config_name)
    shards = sorted(args.dir.glob("labels-*.npz"), key=lambda path: (path.stat().st_mtime, path.name))
    for start in range(0, len(shards), args.batch):
        batch = shards[start:start + args.batch]
        size = sum(path.stat().st_size for path in batch)
        began = time.monotonic()
        if not upload(args, [path.name for path in batch]):
            return False
        seconds = max(time.monotonic() - began, 1e-6)
        for path in batch:
            archive(args, path, sent_dir)
        totals["files"] += len(batch)
        totals["bytes"] += size
        print(f"Sent {len(batch)} shards ({size / 2 ** 20:.1f} MB, {size / 2 ** 20 / seconds:.1f} MB/s); "
              f"total {totals['files']} shards, {totals['bytes'] / 2 ** 20:.1f} MB", flush=True)
    return True


def main(argv=None) -> dict:
    args = parse_args(argv)
    args.dir.mkdir(parents=True, exist_ok=True)
    totals = dict(files=0, bytes=0)
    with output_lock(args.dir, ".send.lock", "label sender"):
        print(f"Sending {args.dir} -> {args.to} every {args.interval:g} s", flush=True)
        while not remote(args, f"mkdir -p {shlex.quote(args.remote + '/.incoming')}", "mkdir"):
            if args.once:
                return totals
            time.sleep(args.interval)
        seen_running = False
        try:
            while True:
                # Read before the pass, so the pass sees every shard written before the maker let go of its lock.
                running = maker_running(args.dir)
                seen_running |= running
                stop = (args.dir / "STOP_SEND").exists()
                complete = send_pass(args, totals)
                if args.once or (complete and ((seen_running and not running) or stop)):
                    break
                time.sleep(args.interval)
        except KeyboardInterrupt:
            print("Interrupted", flush=True)
    print(f"Done: {totals['files']} shards, {totals['bytes'] / 2 ** 20:.1f} MB sent", flush=True)
    return totals


if __name__ == "__main__":
    main()
