"""Measure the Expansion reference and fused path on 72 cells and 72 x 96 layer-2 cells.

From crates/pyengine: nice -n 10 ../../.venv-expand/bin/python search2_expand_bench.py --repeats 30
No network/model inference is timed. Both paths retain battles and both observation trackers, encode
both policy rows and compute both request and engine masks. Layer 2 uses the existing SearchGrid.run.
"""
from __future__ import annotations

import argparse
import gc
import json
import os
from pathlib import Path
import sys
from time import perf_counter

os.environ.setdefault("NUMBRION_SEARCH_PREREQS_DONE", "1")
sys.path.insert(0, str(Path(__file__).resolve().parent / "training_tests"))
import numpy as np
import numbrion as nb
from test_search2_expand import DEX, cells, check, reference, teams, tracker_pair, play_step


def main(repeats):
    rng = np.random.default_rng(518)
    pool = teams()
    roots, trackers = [], []
    for game in range(4):
        battle = nb.Battle((game + 1, 33, 77, 88), pool[2 * game], pool[2 * game + 1], log=True)
        battle.start()
        pair = tracker_pair(battle)
        for _ in range(3):
            play_step(battle, rng)
            pair = tuple(pair[s].feed(battle.drain_player_log(s)) for s in range(2))
        roots.append(battle)
        trackers.append(pair)
    actions = np.concatenate([cells(root, 18, rng) for root in roots])
    world = np.repeat(np.arange(4), 18)
    seeds = rng.integers(1 << 16, size=(72, 4))
    grid, executor = nb.SearchGrid(DEX, 4), nb.GridExecutor(4)
    kwargs = dict(world=world, seeds=seeds)
    fused = lambda: grid.expand(roots, actions, trackers, **kwargs, allow_live=True)
    slow = lambda: reference(grid, executor, roots, actions, trackers, **kwargs)
    expanded = fused()
    check(grid, expanded, slow())
    live = np.flatnonzero(~expanded["ended"])
    assert len(live) == 72, f"benchmark needs 72 live leaves, got {len(live)}"
    leaves = [expanded["leaves"][i] for i in live]
    states = [expanded["trackers"][i] for i in live]
    actions2 = np.concatenate([cells(leaf, 96, rng) for leaf in leaves])
    world2 = np.repeat(np.arange(len(leaves)), 96)
    seeds2 = rng.integers(1 << 16, size=(len(actions2), 4))

    def layer2():
        return grid.run(leaves, actions2, states, 0, world=world2, seeds=seeds2, allow_live=True)

    def pipeline(expand):
        out = expand()
        return grid.run([out["leaves"][i] for i in live], actions2,
                        [out["trackers"][i] for i in live], 0,
                        world=world2, seeds=seeds2, allow_live=True)

    raw = executor.run(roots, actions, **kwargs, logs=True, requests=True, keep_leaves=True, allow_live=True)
    root_states = [trackers[int(world[i])][s] for i in range(72) for s in range(2)]
    logs = [raw["logs"][i][s] for i in range(72) for s in range(2)]
    requests = [raw["requests"][i][s] for i in range(72) for s in range(2)]

    def feed():
        return [state.feed(lines) for state, lines in zip(root_states, logs)]

    fed = feed()

    def feed_encode():
        return grid.encode_states(feed(), requests)

    operations = {
        "expand_reference_72": slow,
        "expand_fused_72": fused,
        "feed_144": feed,
        "encode_144": lambda: grid.encode_states(fed, requests),
        "feed_encode_144": feed_encode,
        "layer2_run_6912": layer2,
        "pipeline_reference": lambda: pipeline(slow),
        "pipeline_fused": lambda: pipeline(fused),
    }
    for fn in operations.values():
        fn()
    durations = {name: [] for name in operations}
    gc.collect()
    gc.disable()
    try:
        for _ in range(repeats):
            # Alternate the order to reduce systematic bias on the shared machine.
            for name in rng.permutation(list(operations)):
                start = perf_counter()
                output = operations[name]()
                elapsed = (perf_counter() - start) * 1000
                durations[name].append(elapsed)
                del output
    finally:
        gc.enable()
    result = {name: {"mean_ms": float(np.mean(ms)), "median_ms": float(np.median(ms)),
                     "std_ms": float(np.std(ms)), "min_ms": float(np.min(ms))}
              for name, ms in durations.items()}
    print(json.dumps(dict(threads=4, repeats=repeats, expansion_cells=72, policy_rows=144,
                          layer2_cells=len(actions2), results=result), indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=30)
    main(parser.parse_args().repeats)
