"""Observation batches from the Showdown fixtures (data/fixtures/*.jsonl.gz), for the Rust-vs-Python parity checks.

Each fixture battle stores, per decision, the omniscient log since the previous decision and both players'
requests. ``batches`` replays ``envs`` battles in lockstep as rows ``2 * env + side`` and yields, per decision
boundary, the list of ``BatchEnv.observe``-style dicts (``battle_id``, ``request`` JSON text or "", player-view
``log``). A finished battle's env moves on to the next fixture battle (a new ``battle_id`` resets the trackers),
after one observation of the battle's final lines.

Usage: python observations.py OUT.jsonl [--fixtures data/fixtures/sample-50.jsonl.gz ...] [--envs 8]
Writes one JSON line per boundary: {"observations": [...]}.
"""
from __future__ import annotations

import argparse
import gzip
import json
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]


def player_view(lines: list[str], side: int) -> list[str]:
    """``|split|pN`` triples resolved to the secret line for side N and the public line otherwise, empty lines
    dropped (the engine's ``drain_player``)."""
    own = f"p{side + 1}"
    out, i = [], 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("|split|"):
            secret, public = lines[i + 1], lines[i + 2]
            chosen = secret if line[7:] == own else public
            if chosen:
                out.append(chosen)
            i += 3
            continue
        if line:
            out.append(line)
        i += 1
    return out


def battles(paths: list[Path]):
    """(battle_id, list of (log, requests) per decision plus the final (log, {}))."""
    battle_id = 0
    for path in paths:
        with gzip.open(path, "rt", encoding="utf-8") as f:
            for line in f:
                record = json.loads(line)
                battle_id += 1
                steps = [(s["log"], s.get("requests") or {}) for s in record["steps"]]
                steps.append((record["end"].get("log", []), {}))
                yield battle_id, steps


def batches(paths: list[Path], envs: int, limit: int | None = None):
    source = battles(paths)
    current = [next(source, None) for _ in range(envs)]
    position = [0] * envs
    count = 0
    while any(c is not None for c in current):
        batch = []
        for e in range(envs):
            battle = current[e]
            for side in (0, 1):
                if battle is None:  # no battles left: an empty observation of a battle no row uses
                    batch.append({"battle_id": -1, "request": "", "log": []})
                    continue
                battle_id, steps = battle
                log, requests = steps[position[e]]
                request = requests.get(f"p{side + 1}")
                batch.append({"battle_id": battle_id, "request": json.dumps(request) if request else "",
                              "log": player_view(log, side)})
        yield batch
        count += 1
        if limit is not None and count >= limit:
            return
        for e in range(envs):
            if current[e] is None:
                continue
            position[e] += 1
            if position[e] >= len(current[e][1]):
                current[e], position[e] = next(source, None), 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("out", type=Path)
    parser.add_argument("--fixtures", type=Path, nargs="+", default=[REPO / "data/fixtures/sample-50.jsonl.gz"])
    parser.add_argument("--envs", type=int, default=8)
    parser.add_argument("--limit", type=int, default=None, help="at most this many boundaries")
    args = parser.parse_args()
    n = 0
    with open(args.out, "w", encoding="utf-8") as f:
        for batch in batches(args.fixtures, args.envs, args.limit):
            f.write(json.dumps({"observations": batch}) + "\n")
            n += 1
    print(f"{n} boundaries x {2 * args.envs} rows -> {args.out}")


if __name__ == "__main__":
    main()
