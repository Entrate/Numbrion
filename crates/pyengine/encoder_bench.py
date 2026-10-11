"""Time observation encoding and scripted play on real self-play boundaries.

    python crates/pyengine/encoder_bench.py --envs 32 --boundaries 256 [--pool FILE ...]

Batch encoding excludes Python observe(); encode_env includes draining Rust player views. The additional
observe+Python row is the end-to-end comparison for encode_env. Smart timings include both slots and use
the same observations and rows, with engine masks conditioned on each player's first action. Engine
stepping and mask creation are not timed.
"""
from __future__ import annotations

import argparse
import time

import numpy as np
import torch

import numbrion as nb
from bench import default_pool
from training.encoder import PyEncoder, RustEncoder
from training.smart import PySmartHeuristicAgent, RustSmartHeuristicAgent


def positive(value):
    value = int(value)
    if value <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return value


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--pool", nargs="+", help="packed team pool files")
    ap.add_argument("--envs", type=positive, default=32)
    ap.add_argument("--boundaries", type=positive, default=256)
    ap.add_argument("--threads", type=positive, default=1)
    args = ap.parse_args()
    if not hasattr(nb, "ObsEncoder") or not hasattr(nb, "SmartAgent"):
        ap.error("install a numbrion wheel with ObsEncoder and SmartAgent")
    torch.set_num_threads(1)
    pool = args.pool or [str(p) for p in default_pool()]
    env = nb.BatchEnv(args.envs, pool, seed=7, threads=args.threads, log=True)
    fast_env = nb.BatchEnv(args.envs, pool, seed=7, threads=args.threads, log=True)
    result = env.reset()
    fast_env.reset()
    nrows = 2 * args.envs
    py_encoder, rs_encoder, fast_encoder = PyEncoder(nrows), RustEncoder(nrows), RustEncoder(nrows)
    py_smart = PySmartHeuristicAgent(np.random.default_rng(3))
    rs_smart = RustSmartHeuristicAgent(np.random.default_rng(3))
    elapsed = dict(python_batch=0.0, rust_batch=0.0, rust_env=0.0, observe_python=0.0, python_smart=0.0, rust_smart=0.0)
    smart_rows = 0

    def timed(key, f, measured):
        started = time.perf_counter()
        out = f()
        if measured:
            elapsed[key] += time.perf_counter() - started
        return out

    # Warm tables/caches without including their one-off costs in the per-row timings.
    for boundary in range(args.boundaries + 5):
        measured = boundary >= 5
        started = time.perf_counter()
        observations = [env.observe(e, s) for e in range(args.envs) for s in (0, 1)]
        buffers = timed("python_batch", lambda: py_encoder.encode_batch(observations), measured)
        if measured:
            elapsed["observe_python"] += time.perf_counter() - started
        timed("rust_batch", lambda: rs_encoder.encode_batch(observations), measured)
        timed("rust_env", lambda: fast_encoder.encode_env(fast_env), measured)
        arrays = buffers.arrays()
        rows = np.flatnonzero(result["needs_action"].reshape(-1))
        if len(rows):
            mask0 = result["mask0"].reshape(-1, 47)[rows]
            py0 = timed("python_smart", lambda: py_smart.first(arrays, rows, mask0), measured)
            rs0 = timed("rust_smart", lambda: rs_smart.first(arrays, rows, mask0), measured)
            a0 = np.full(nrows, nb.PASS, dtype=np.int32)
            a0[rows] = py0
            mask1 = env.mask_slot1(a0.reshape(args.envs, 2)).reshape(-1, 47)[rows]
            timed("python_smart", lambda: py_smart.second(rows, mask1), measured)
            a0[rows] = rs0
            rs_mask1 = env.mask_slot1(a0.reshape(args.envs, 2)).reshape(-1, 47)[rows]
            timed("rust_smart", lambda: rs_smart.second(rows, rs_mask1), measured)
            if measured:
                smart_rows += len(rows)
        actions = env.random_actions()
        result = env.step(actions)
        fast_env.step(actions)
    print(f"numbrion {nb.__version__}; {args.envs} envs; {args.boundaries} measured boundaries; {args.threads} engine threads")
    encoding_rows = args.boundaries * nrows
    for key in ("python_batch", "rust_batch", "observe_python", "rust_env"):
        print(f"{key:16s} {1000 * elapsed[key] / encoding_rows:.6f} ms/row")
    print(f"encode_batch speed-up: {elapsed['python_batch'] / elapsed['rust_batch']:.2f}x")
    print(f"encode_env speed-up vs observe+Python: {elapsed['observe_python'] / elapsed['rust_env']:.2f}x")
    if smart_rows:
        for key in ("python_smart", "rust_smart"):
            print(f"{key:16s} {1000 * elapsed[key] / smart_rows:.6f} ms/row (both slots)")
        print(f"smart speed-up: {elapsed['python_smart'] / elapsed['rust_smart']:.2f}x")


if __name__ == "__main__":
    main()
