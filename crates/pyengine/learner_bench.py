"""Time the self-play learner (one PPO update) on a saved rollout, without actors running.

Run from the repository root:

    .venv/bin/python crates/pyengine/learner_bench.py --collect scratch/bench/rollout.npz
    .venv/bin/python crates/pyengine/learner_bench.py --rollout scratch/bench/rollout.npz --device mps
    .venv/bin/python crates/pyengine/learner_bench.py --rollout scratch/bench/rollout.npz --device cpu --threads 4

``--collect`` plays one rollout with the default trainer settings and an untrained policy and saves the buffer.
The timing runs ``--repeat`` full ``ppo_update`` calls after one warm-up update and reports seconds per update and
trained samples per second; ``--breakdown`` adds one update timed phase by phase (synchronizing after each phase,
so its total is a little higher; DirectML has no synchronize call, so its phase times are not meaningful).
"""
from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

import numpy as np
import torch

import train_selfplay as ts
from training.actors import Actors
from training.device import BACKENDS, training_device
from training.dex import load_dex
from training.league import opponent_kinds
from training.model import Model
from training.ppo import Adam, clip_gradients, minibatches

REPO = Path(__file__).resolve().parents[2]


def collect(args) -> None:
    torch.manual_seed(args.seed)
    model = Model(load_dex(), args.width, args.layers)
    kinds = opponent_kinds(args.envs, 0.3, 0.33)
    with Actors(args.envs, args.steps, [args.pool], args.seed, args.workers, model, scripted="smart",
                scripted_temperature=0.05) as actors:  # the trainer's default opponents
        actors.set_weights("learner", ts.cpu_state(model))
        actors.start(0, np.where(kinds == 1, 0, kinds).astype(np.int8), 0.02)  # no snapshot yet: those envs self-play
        actors.wait()
        arrays = {k: np.array(v) for k, v in actors.buffers[0].arrays.items()}
    args.collect.parent.mkdir(parents=True, exist_ok=True)
    np.savez(args.collect, **arrays)
    print(json.dumps({"saved": str(args.collect), "trained_rows": int(arrays["train"].sum())}))


def sync(device) -> None:
    if device.type == "mps":
        torch.mps.synchronize()
    elif device.type == "cuda":
        torch.cuda.synchronize()


def breakdown(model, optimizer, data, args, device) -> dict:
    """One update's minibatch loop from ts.ppo_update (eager loss), synchronized after each phase."""
    phases = dict(prepare=0.0, upload=0.0, forward=0.0, backward=0.0, clip=0.0, step=0.0)
    t = time.perf_counter()
    steps, R = data["a0"].shape
    advantages, returns = ts.gae(data["reward"], data["value"], data["done"], data["bootstrap"], args.gamma, args.lam)
    selected = np.flatnonzero(data["train"].reshape(-1))
    flat = {k: data[k].reshape(-1, *data[k].shape[2:]) for k in ts.OBS_KEYS}
    partner_index = (np.arange(steps)[:, None] * R + (np.arange(R) ^ 1)[None, :]).reshape(-1)
    adv = advantages.reshape(-1)[selected]
    adv = ((adv - adv.mean()) / (adv.std() + 1e-8)).astype(np.float32)
    columns = dict(flat, a0=data["a0"].reshape(-1), a1=data["a1"].reshape(-1), mask0=data["mask0"].reshape(-1, 47),
                   mask1=data["mask1"].reshape(-1, 47), logp=data["logp"].reshape(-1),
                   returns=returns.reshape(-1).astype(np.float32))
    coefs = [torch.full((), x, device=device) for x in (args.value_coef, 0.01, args.aux_coef)]
    rng = np.random.default_rng(0)

    def up(array):
        return torch.from_numpy(np.ascontiguousarray(array)).to(device)

    resident = device.type == "mps"  # as in ts.ppo_update
    if resident:
        columns, adv = {k: up(v) for k, v in columns.items()}, up(adv)
    sync(device)
    phases["prepare"] = time.perf_counter() - t
    for _ in range(args.epochs):
        for pick in minibatches(rng.permutation(len(selected)), args.minibatch):
            t = time.perf_counter()
            index, partner_rows = selected[pick], partner_index[selected[pick]]
            if resident:
                pick, index, partner_rows = up(pick), up(index), up(partner_rows)
            get = (lambda k, i, *extra: columns[k][(i, *extra)]) if resident else (lambda k, i, *extra: up(columns[k][(i, *extra)]))
            inputs = ({k: get(k, index) for k in ts.OBS_KEYS}, get("ids", partner_rows), get("floats", partner_rows, slice(0, 6)),
                      get("a0", index), get("a1", index), get("mask0", index), get("mask1", index), get("logp", index),
                      adv[pick] if resident else up(adv[pick]), get("returns", index))
            sync(device)
            phases["upload"] += time.perf_counter() - t
            t = time.perf_counter()
            loss, _ = ts.ppo_loss(model, *inputs, *coefs, args.clip)
            sync(device)
            phases["forward"] += time.perf_counter() - t
            t = time.perf_counter()
            optimizer.zero_grad()
            loss.backward()
            sync(device)
            phases["backward"] += time.perf_counter() - t
            t = time.perf_counter()
            norm = clip_gradients(optimizer.parameters, args.max_grad_norm)
            sync(device)
            phases["clip"] += time.perf_counter() - t
            t = time.perf_counter()
            optimizer.step(skip=~torch.isfinite(norm))
            sync(device)
            phases["step"] += time.perf_counter() - t
    return {k: round(v, 3) for k, v in phases.items()}


def bench(args) -> None:
    if args.threads:
        torch.set_num_threads(args.threads)
    torch.manual_seed(args.seed)
    device, name = training_device(args.device)
    data = dict(np.load(args.rollout))
    model = Model(load_dex(), args.width, args.layers).to(device)
    optimizer = Adam(model.parameters(), lr=3e-4)
    rng = np.random.default_rng(args.seed)
    model.train()
    times, samples = [], 0
    for i in range(args.repeat + 1):
        sync(device)
        t = time.perf_counter()
        _, samples = ts.ppo_update(model, optimizer, data, args, rng, 0.01, device)
        sync(device)
        if i:  # the first update compiles kernels
            times.append(time.perf_counter() - t)
    out = {"device": name, "threads": torch.get_num_threads(), "minibatch": args.minibatch, "compile": args.compile,
           "samples": samples,
           "update_s": round(float(np.median(times)), 3), "samples_per_s": round(samples / float(np.median(times)))}
    if args.breakdown:
        out["phases"] = breakdown(model, optimizer, data, args, device)
    print(json.dumps(out), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--collect", type=Path, help="play one rollout and save it here")
    parser.add_argument("--rollout", type=Path, default=REPO / "scratch/bench/rollout.npz")
    parser.add_argument("--pool", type=Path, default=REPO / "data/teams/train-s42-2000.txt")
    parser.add_argument("--device", choices=BACKENDS, default="auto")
    parser.add_argument("--threads", type=int, default=0, help="torch CPU threads (0: torch's default)")
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--breakdown", action="store_true")
    parser.add_argument("--compile", action="store_true", help="torch.compile the minibatch loss (as the trainer's flag)")
    parser.add_argument("--envs", type=int, default=128)
    parser.add_argument("--steps", type=int, default=32)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--width", type=int, default=128)
    parser.add_argument("--layers", type=int, default=3)
    parser.add_argument("--epochs", type=int, default=2)
    parser.add_argument("--minibatch", type=int, default=1024)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()
    # The PPO settings ppo_update reads, at the trainer's defaults.
    args.gamma, args.lam, args.clip, args.value_coef, args.aux_coef, args.max_grad_norm = 0.995, 0.95, 0.2, 0.5, 0.03, 0.5
    args.tera_entropy_weight = 0.0
    if args.collect:
        collect(args)
    else:
        bench(args)


if __name__ == "__main__":
    main()
