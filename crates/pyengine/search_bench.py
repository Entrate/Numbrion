"""One-decision grid search throughput: GridExecutor vs a per-cell Python loop.

    python crates/pyengine/search_bench.py [--roots 32] [--threads 1 3] [--seconds 1] [--rounds 4] [--log]

Roots are mid-battle turn boundaries (turn 3-10 of random self-play on the test pool) where both sides choose;
each gets a 10 x 10 grid (10 legal joint actions per side, nb.grid_actions) and one seed per cell. Every cell
is reseeded and returns both sides' request JSON (and with ``--log`` the player-view lines since the root).

* ``executor/N``  GridExecutor(N).run(root, actions, seeds=...) per root (100 cells per call, GIL released).
* ``py-clone``    per cell from Python: root.clone(), reseed, choose_action x2, request_json x2.
* ``py-restore``  the same on one reused worker battle: worker.restore_from(root) instead of clone().

Modes run in alternating order for ``--rounds`` rounds of ``--seconds`` each; the per-round figures show noise.
"""

from __future__ import annotations

import argparse
import gzip
import time
from pathlib import Path

import numpy as np

import numbrion as nb

HERE = Path(__file__).resolve().parent


def pairs(b, side, rng, k=10):
    m0, joint = b.legal_mask(side)
    out = []
    for _ in range(200):
        a0 = int(rng.choice(np.flatnonzero(m0)))
        p = (a0, int(rng.choice(np.flatnonzero(joint[a0]))))
        if p not in out:
            out.append(p)
        if len(out) == k:
            break
    return [out[i % len(out)] for i in range(k)]


def make_roots(teams, count, log, rng):
    roots = []
    while len(roots) < count:
        i, j = rng.integers(len(teams), size=2)
        b = nb.Battle(tuple(int(x) for x in rng.integers(1 << 16, size=4)), teams[i], teams[j], log=log)
        b.start()
        target = int(rng.integers(3, 11))
        while not b.ended and b.turn < target:
            for side in [s for s in (0, 1) if b.needs_action(s)]:
                p = pairs(b, side, rng, 1)[0]
                b.choose_action(side, *p)
        if b.ended or b.request_kind(0) != "move" or b.request_kind(1) != "move":
            continue
        actions = nb.grid_actions(pairs(b, 0, rng), pairs(b, 1, rng))
        seeds = rng.integers(1 << 16, size=(len(actions), 4))
        if log:
            b.drain_log(), b.drain_player_log(0), b.drain_player_log(1)
        roots.append((b, actions, seeds, [tuple(int(x) for x in s) for s in seeds]))
    return roots


def python_cells(roots, worker, log):
    n = 0
    for root, actions, _, seeds in roots:
        for i in range(len(actions)):
            if worker is None:
                c = root.clone()
            else:
                c = worker
                c.restore_from(root)
            c.reseed(seeds[i])
            c.choose_action(0, int(actions[i, 0, 0]), int(actions[i, 0, 1]))
            c.choose_action(1, int(actions[i, 1, 0]), int(actions[i, 1, 1]))
            c.request_json(0), c.request_json(1)
            if log:
                c.drain_player_log(0), c.drain_player_log(1)
            n += 1
    return n


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--roots", type=int, default=32)
    ap.add_argument("--threads", type=int, nargs="+", default=[1, 3])
    ap.add_argument("--seconds", type=float, default=1.0)
    ap.add_argument("--rounds", type=int, default=4)
    ap.add_argument("--log", action="store_true")
    args = ap.parse_args()
    with gzip.open(HERE / "testdata" / "pool-400.txt.gz", "rt", encoding="utf-8") as f:
        teams = f.read().splitlines()
    rng = np.random.default_rng(2026)
    roots = make_roots(teams, args.roots, args.log, rng)
    executors = {t: nb.GridExecutor(t) for t in args.threads}
    worker = roots[0][0].clone()

    def run(mode):
        if mode.startswith("executor/"):
            ex = executors[int(mode.split("/")[1])]
            n = 0
            for root, actions, seeds, _ in roots:
                n += len(ex.run(root, actions, seeds=seeds)["turn"])
            return n
        return python_cells(roots, worker if mode == "py-restore" else None, args.log)

    modes = [f"executor/{t}" for t in args.threads] + ["py-restore", "py-clone"]
    totals = {m: [0, 0.0, []] for m in modes}
    print(f"{len(roots)} roots x 100 cells, log={args.log}, {args.rounds} rounds x {args.seconds} s per mode")
    for r in range(args.rounds):
        for m in modes if r % 2 == 0 else modes[::-1]:
            t0, n = time.perf_counter(), 0
            while time.perf_counter() - t0 < args.seconds:
                n += run(m)
            dt = time.perf_counter() - t0
            totals[m][0] += n
            totals[m][1] += dt
            totals[m][2].append(dt * 1e6 / n)
    for m in modes:
        n, dt, per = totals[m]
        rounds = " ".join(f"{x:.1f}" for x in per)
        print(f"{m:>12}: {n / dt:8.0f} cells/s {dt * 1e6 / n:8.2f} us/cell  (rounds: {rounds})")


if __name__ == "__main__":
    main()
