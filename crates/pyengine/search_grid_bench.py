"""Search leaf evaluation throughput: GridExecutor + Python-side ObsEncoder.encode_batch vs the fused SearchGrid.

    python crates/pyengine/search_grid_bench.py [--positions 8] [--threads 1 2 4 6] [--seconds 1] [--rounds 3]

Positions are mid-battle turn boundaries (turn 3-10 of random self-play on the test pool) where both sides choose.
Each decision is one call: 4 worlds (copies of the position, each reseeded: the engine work of a WorldBuilder
world) x 8 of our legal pairs x 8 of the opponent's = 256 cells, one seed per cell. The observation trackers of
both sides come from an ObsEncoder fed the whole history; side 0 is the searching player.

* ``slow/N``   GridExecutor(N).run(..., logs, requests), then the leaves encoded in Python: one ObsEncoder with
  two rows per cell, every row restored from the root snapshots (``ObsEncoder.restore``), one ``encode_batch``
  of all leaf observations (damage features included), and the eval/partner rows gathered with numpy.
* ``fused/N``  SearchGrid(N).run(...): the same cells with the eval side's observation (ids, floats, field,
  token features) and the partner's own tokens encoded on the Rust threads.
* ``grid/N``   GridExecutor(N).run(...) alone (logs and requests returned, nothing encoded): the engine part.

Modes run in alternating order for ``--rounds`` rounds of ``--seconds`` each; the spread of the per-round figures
shows the noise of a shared machine.
"""

from __future__ import annotations

import argparse
import gzip
import os
import time
import warnings
from pathlib import Path

import numpy as np

import numbrion as nb

# A benchmark on copies of live battles: the search reminders do not apply.
warnings.filterwarnings("ignore", message="numbrion.(GridExecutor|SearchGrid)")

HERE = Path(__file__).resolve().parent
DEX = HERE.parents[1] / "data" / "training" / "dex.json"
L = nb.ObsEncoder.layout()
ORDER = ("ids", "floats", "field", "foe_match", "fainted", "guard", "action_features", "token_features")


def buffers(rows):
    return dict(
        ids=np.zeros((rows, L["N_TOKENS"], L["N_IDS"]), np.int64),
        floats=np.zeros((rows, L["N_TOKENS"], L["N_POKEMON_FLOATS"]), np.float32),
        field=np.zeros((rows, L["N_FIELD_FLOATS"]), np.float32),
        foe_match=np.zeros((rows, 6), np.int64),
        fainted=np.zeros((rows, 2), np.float32),
        guard=np.zeros((rows, 2, 4, 2), bool),
        action_features=np.zeros((rows, 2, L["N_CODES"], L["N_ACTION_FEATURES"]), np.float32),
        token_features=np.zeros((rows, L["N_TOKENS"], L["N_TOKEN_FEATURES"]), np.float32),
    )


def pairs(b, side, rng, k):
    m0, joint = b.legal_mask(side)
    out = []
    for _ in range(200):
        a0 = int(rng.choice(np.flatnonzero(m0)))
        p = (a0, int(rng.choice(np.flatnonzero(joint[a0]))))
        if p not in out:
            out.append(p)
        if len(out) == k:
            break
    return np.array([out[i % len(out)] for i in range(k)], dtype=np.int32)


def make_positions(teams, count, rng, worlds, k):
    """Decisions: (worlds, actions, world index per cell, seeds, (side-0 state, side-1 state))."""
    out = []
    while len(out) < count:
        i, j = rng.integers(len(teams), size=2)
        b = nb.Battle(tuple(int(x) for x in rng.integers(1 << 16, size=4)), teams[i], teams[j], log=True)
        b.start()
        enc = nb.ObsEncoder(str(DEX), 2)
        bufs = buffers(2)

        def observe():
            obs = [{"battle_id": 0, "request": b.request_json(s), "log": b.drain_player_log(s)} for s in (0, 1)]
            enc.encode_batch(obs, *(bufs[key] for key in ORDER))
            b.drain_log()

        observe()
        target = int(rng.integers(3, 11))
        while not b.ended and b.turn < target:
            acting = [s for s in (0, 1) if b.needs_action(s)]
            for s in acting:
                b.choose_action(s, *map(int, pairs(b, s, rng, 1)[0]))
            observe()
        if b.ended or b.request_kind(0) != "move" or b.request_kind(1) != "move":
            continue
        roots = [b.clone() for _ in range(worlds)]
        for w in roots:
            w.reseed(tuple(int(x) for x in rng.integers(1 << 16, size=4)))
        cells = [nb.grid_actions(pairs(w, 0, rng, k), pairs(w, 1, rng, k)) for w in roots]
        actions = np.concatenate(cells)
        world = np.concatenate([np.full(len(c), n) for n, c in enumerate(cells)])
        seeds = rng.integers(1 << 16, size=(len(actions), 4))
        out.append((roots, actions, world, seeds, (enc.snapshot(0), enc.snapshot(1))))
    return out


class Slow:
    """GridExecutor plus Python-side encoding of the leaves with ObsEncoder.encode_batch."""

    def __init__(self, threads, cells):
        self.ex = nb.GridExecutor(threads)
        self.enc = nb.ObsEncoder(str(DEX), 2 * cells)
        self.bufs = buffers(2 * cells)
        self.cells = cells

    def run(self, position, prefer=0):
        roots, actions, world, seeds, (s0, s1) = position
        out = self.ex.run(roots, actions, world=world, seeds=seeds, allow_live=True)
        n = len(actions)
        assert n == self.cells
        observations = []
        for i in range(n):
            self.enc.restore(2 * i, s0)
            self.enc.restore(2 * i + 1, s1)
            for s in (0, 1):
                observations.append({"battle_id": 0, "request": out["requests"][i][s], "log": out["logs"][i][s]})
        self.enc.encode_batch(observations, *(self.bufs[key] for key in ORDER))
        needs = out["needs_action"]
        eval_side = np.where(needs[:, prefer], prefer, np.where(needs[:, 1 - prefer], 1 - prefer, -1))
        rows = 2 * np.arange(n) + np.maximum(eval_side, 0)
        partner = rows ^ 1
        b = self.bufs
        return dict(eval_side=eval_side, ids=b["ids"][rows], floats=b["floats"][rows], field=b["field"][rows],
                    token_features=b["token_features"][rows], partner_ids=b["ids"][partner, 0:6],
                    partner_floats=b["floats"][partner, 0:6])


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--positions", type=int, default=8)
    ap.add_argument("--worlds", type=int, default=4)
    ap.add_argument("--pairs", type=int, default=8, help="candidate pairs per side")
    ap.add_argument("--threads", type=int, nargs="+", default=[1, 2, 4, 6])
    ap.add_argument("--seconds", type=float, default=1.0)
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--modes", nargs="+", default=["grid", "slow", "fused"])
    args = ap.parse_args()
    with gzip.open(HERE / "testdata" / "pool-400.txt.gz", "rt", encoding="utf-8") as f:
        teams = f.read().splitlines()
    rng = np.random.default_rng(2026)
    positions = make_positions(teams, args.positions, rng, args.worlds, args.pairs)
    cells = len(positions[0][1])
    runners = {}
    for t in args.threads:
        if "grid" in args.modes:
            ex = nb.GridExecutor(t)
            runners[f"grid/{t}"] = lambda p, ex=ex: ex.run(p[0], p[1], world=p[2], seeds=p[3], allow_live=True)
        if "slow" in args.modes:
            runners[f"slow/{t}"] = Slow(t, cells).run
        if "fused" in args.modes:
            grid = nb.SearchGrid(DEX, t)
            runners[f"fused/{t}"] = lambda p, g=grid: g.run(p[0], p[1], p[4], 0, world=p[2], seeds=p[3], allow_live=True)
    # The two paths agree (the tests check every array bit for bit; here a spot check).
    if "slow" in args.modes and "fused" in args.modes:
        a, b = runners[f"slow/{args.threads[0]}"](positions[0]), runners[f"fused/{args.threads[0]}"](positions[0])
        for key in ("eval_side", "ids", "floats", "token_features", "partner_floats"):
            assert np.array_equal(a[key], b[key]), key
    modes = list(runners)
    totals = {m: [0, 0.0, []] for m in modes}
    print(f"{len(positions)} decisions x {cells} cells ({args.worlds} worlds x {args.pairs} x {args.pairs} pairs), "
          f"{args.rounds} rounds x {args.seconds} s per mode, {os.cpu_count()} CPUs")
    for r in range(args.rounds):
        for m in modes if r % 2 == 0 else modes[::-1]:
            t0, n = time.perf_counter(), 0
            while time.perf_counter() - t0 < args.seconds:
                for p in positions:
                    runners[m](p)
                    n += cells
            dt = time.perf_counter() - t0
            totals[m][0] += n
            totals[m][1] += dt
            totals[m][2].append(dt * 1e6 / n)
    print(f"{'mode':10} {'us/cell':>8} {'cells/s':>9}  per round (us/cell)")
    for m in modes:
        n, dt, per = totals[m]
        print(f"{m:10} {dt * 1e6 / n:8.1f} {n / dt:9.0f}  " + " ".join(f"{x:.1f}" for x in per))


if __name__ == "__main__":
    main()
