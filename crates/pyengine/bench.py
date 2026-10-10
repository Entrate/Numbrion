"""Random legal self-play throughput of the numbrion training interface.

    python crates/pyengine/bench.py [--pool FILE ...] [--threads 1 6] [--seconds 8] [--envs 64]

Three levels are measured for each thread count:

* ``rust``   BatchEnv.run_random: the whole loop (sampling, masks, choose, engine, auto-reset) stays in Rust;
             the ceiling for what a Python driver can reach.
* ``step``   a Python loop calling env.step(env.random_actions()): adds the per-step API cost (numpy result
             arrays, GIL hand-off) with the sampler still in Rust.
* ``numpy``  the same loop with the actions sampled in numpy from the masks (nb.sample_uniform): a policy
             that lives in Python pays at least this.

``decisions`` counts side choices (one per acting side per boundary), ``boundaries`` the decision points of a
battle. ``--log`` repeats the ``step`` level with TextLog battles (player log views available to observe()).
Team pools are sampled uniformly (default: the s1 pool under data/teams if present, else the small test pool).
"""

from __future__ import annotations

import argparse
import time
from pathlib import Path

import numpy as np

import numbrion as nb

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]


def default_pool() -> list:
    for cand in (REPO / "data" / "teams" / "pool-s1-200k.txt.gz",):
        if cand.exists():
            return [cand]
    return [HERE / "testdata" / "pool-400.txt.gz"]


def fmt(label: str, threads: int, envs: int, battles: int, decisions: int, boundaries: int, secs: float) -> None:
    print(
        f"{label:<7} threads={threads:<2} envs={envs:<4} "
        f"{battles / secs:8.0f} battles/s  {decisions / secs:9.0f} decisions/s  "
        f"{boundaries / secs:9.0f} boundaries/s  ({battles / secs / threads:6.0f} battles/s/thread)  "
        f"[{battles} battles in {secs:.1f}s]"
    )


def bench_rust(pool, threads, envs, seconds):
    env = nb.BatchEnv(envs, pool, seed=1, threads=threads, max_teams_per_file=20000)
    env.run_random(1)  # warm-up
    # Calibrate battles per env so the timed run takes about `seconds`.
    t = time.perf_counter()
    cal = env.run_random(2)
    per_battle = (time.perf_counter() - t) / max(cal["battles"], 1)
    n = max(1, round(seconds / (per_battle * envs)))
    s = env.run_random(n)
    fmt("rust", threads, envs, s["battles"], s["decisions"], s["boundaries"], s["seconds"])
    return s["battles"] / s["seconds"]


def bench_loop(label, pool, threads, envs, seconds, sampler, log=False):
    env = nb.BatchEnv(envs, pool, seed=1, threads=threads, log=log, max_teams_per_file=20000)
    rng = np.random.default_rng(0)
    r = env.reset()

    def act(r):
        return env.random_actions() if sampler == "rust" else nb.sample_uniform(r, env, rng)

    for _ in range(20):  # warm-up
        r = env.step(act(r))
    battles = decisions = steps = 0
    t = time.perf_counter()
    while time.perf_counter() - t < seconds:
        r = env.step(act(r))
        battles += int(r["done"].sum())
        decisions += r["decisions"]
        steps += 1
    secs = time.perf_counter() - t
    fmt(label, threads, envs, battles, decisions, steps * envs, secs)
    return battles / secs


def bench_clone(pool):
    teams = nb.BatchEnv(1, pool, seed=3, threads=1, max_teams_per_file=100)
    teams.reset()
    b = teams.clone_battle(0)
    n = 300
    t = time.perf_counter()
    for _ in range(n):
        b.clone()
    clone = (time.perf_counter() - t) / n
    c = b.clone()
    n = 20000
    t = time.perf_counter()
    for _ in range(n):
        c.copy_from(b)
    copy = (time.perf_counter() - t) / n
    t = time.perf_counter()
    for _ in range(n):
        b.legal_mask(0)
    mask = (time.perf_counter() - t) / n
    print(f"Battle.clone() {clone * 1e6:.0f} us, copy_from() {copy * 1e6:.1f} us, legal_mask() {mask * 1e6:.1f} us")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pool", nargs="*", type=Path, default=None, help="packed team pool file(s)")
    ap.add_argument("--threads", nargs="*", type=int, default=[1, 6])
    ap.add_argument("--seconds", type=float, default=8.0)
    ap.add_argument("--envs", type=int, default=0, help="environments (default 16 per thread)")
    ap.add_argument("--log", action="store_true", help="also run the step level with TextLog battles")
    args = ap.parse_args()
    pool = args.pool or default_pool()
    print(f"numbrion {nb.__version__}; pool: {', '.join(str(p) for p in pool)}")
    for threads in args.threads:
        envs = args.envs or 16 * threads
        bench_rust(pool, threads, envs, args.seconds)
        bench_loop("step", pool, threads, envs, args.seconds, "rust")
        bench_loop("numpy", pool, threads, envs, args.seconds, "numpy")
        if args.log:
            bench_loop("step+log", pool, threads, envs, args.seconds, "rust", log=True)
    bench_clone(pool)


if __name__ == "__main__":
    main()
