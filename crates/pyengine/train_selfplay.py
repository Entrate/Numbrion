"""Time-budgeted PPO self-play with the transformer policy (docs/training/TIPS.md, docs/training/SELFPLAY.md).

Run from the repository root:

    .venv\\Scripts\\python.exe crates\\pyengine\\train_selfplay.py --minutes 60

* Observations: training.encoder (own request + player-view protocol only), damage features (training.damage).
* Model: training.model (token transformer, per-action scoring, slot b conditioned on slot a, oracle critic,
  auxiliary hidden-information heads).
* Rollouts: actor processes (training.actors) play the battles with CPU copies of the policy and fill one of two
  shared-memory buffers while the GPU trains on the other, so each rollout is collected with the weights from one
  update earlier. PPO's ratio uses the stored behaviour log-probs.
* Opponents: the learner plays both sides of most battles; ``--pool-fraction`` of the battles put a random older
  snapshot (added every ``--pool-minutes``) on side 2 instead. Only the learner's own decisions are trained on.
* Reward: +1 win, -1 loss, plus a KO-difference bonus that decays linearly to zero at half the run.
* Schedule: learning rate and entropy bonus anneal over the time budget; snapshots every ``--snapshot-minutes``
  feed evaluate.py. ``latest.pt`` (with optimizer state) supports ``--resume``.
"""
from __future__ import annotations

import argparse
import json
import os
import time
from pathlib import Path

import numpy as np
import torch

from training.actors import Actors, memory_mb
from training.device import training_device
from training.dex import load_dex
from training.model import Model
from training.ppo import Adam, clip_gradients, gae

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
OBS_KEYS = ("ids", "floats", "field", "foe_match", "action_features", "token_features")
N_ACTIONS = 47


def upload(arrays: dict, index, device) -> dict:
    return {k: torch.from_numpy(np.ascontiguousarray(arrays[k][index])).to(device) for k in OBS_KEYS}


def partner_view(arrays: dict, index, device) -> tuple[torch.Tensor, torch.Tensor]:
    """The opponent rows' own-team ids and floats (oracle critic input)."""
    return (torch.from_numpy(np.ascontiguousarray(arrays["ids"][index])).to(device),
            torch.from_numpy(np.ascontiguousarray(arrays["floats"][index, 0:6])).to(device))


def onehot(actions: torch.Tensor) -> torch.Tensor:
    return (actions.view(-1, 1) == torch.arange(N_ACTIONS, device=actions.device)).float()


def save(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    torch.save(payload, temporary)
    os.replace(temporary, path)


def cpu_state(model: torch.nn.Module) -> dict:
    return {k: v.detach().cpu().clone() for k, v in model.state_dict().items()}


def spread_mask(n: int, fraction: float) -> np.ndarray:
    """``fraction`` of ``n`` entries set, evenly spread (so pool battles are shared across actors)."""
    index = np.arange(n)
    return np.floor((index + 1) * fraction) > np.floor(index * fraction)


def ppo_update(model, optimizer, data: dict, args, rng, entropy_coef: float, device) -> tuple[dict, int]:
    """PPO with the oracle critic and the auxiliary loss on one rollout buffer of [T, rows] arrays."""
    steps, R = data["a0"].shape
    partner = np.arange(R) ^ 1
    advantages, returns = gae(data["reward"], data["value"], data["done"], data["bootstrap"], args.gamma, args.lam)
    selected = np.flatnonzero(data["train"].reshape(-1))
    flat = {k: data[k].reshape(-1, *data[k].shape[2:]) for k in OBS_KEYS}
    partner_index = (np.arange(steps)[:, None] * R + partner[None, :]).reshape(-1)
    adv = advantages.reshape(-1)[selected]
    adv = (adv - adv.mean()) / (adv.std() + 1e-8)
    a0_all, a1_all = data["a0"].reshape(-1), data["a1"].reshape(-1)
    mask0_all, mask1_all = data["mask0"].reshape(-1, N_ACTIONS), data["mask1"].reshape(-1, N_ACTIONS)
    logp_all, returns_all = data["logp"].reshape(-1), returns.reshape(-1)
    stats = {k: [] for k in ("policy_loss", "value_loss", "entropy", "aux_loss", "kl", "clipfrac", "grad_norm")}

    def up(array):
        return torch.from_numpy(np.ascontiguousarray(array)).to(device)

    for _ in range(args.epochs):
        order = rng.permutation(len(selected))
        for begin in range(0, len(order), args.minibatch):
            pick = order[begin:begin + args.minibatch]
            if len(pick) < args.minibatch // 4:
                continue
            index = selected[pick]
            batch = upload(flat, index, device)
            partner_ids, partner_floats = partner_view(flat, partner_index[index], device)
            a0, a1 = up(a0_all[index]), up(a1_all[index])
            state = model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"], batch["token_features"])
            lp0, vectors0 = model.slot0(state, up(mask0_all[index]))
            lp1 = model.slot1(state, vectors0, a0, up(mask1_all[index]))
            new_logp = (lp0 * onehot(a0)).sum(-1) + (lp1 * onehot(a1)).sum(-1)
            old_logp = up(logp_all[index])
            ratio = (new_logp - old_logp).exp()
            weight = up(adv[pick].astype(np.float32))
            policy_loss = -torch.minimum(ratio * weight, ratio.clamp(1 - args.clip, 1 + args.clip) * weight).mean()
            value = model.value(state, partner_ids, partner_floats)
            value_loss = (value - up(returns_all[index].astype(np.float32))).square().mean()
            entropy = -((lp0.exp() * lp0).sum(-1) + (lp1.exp() * lp1).sum(-1)).mean()
            aux_loss = model.aux_loss(state, batch["floats"], batch["foe_match"], partner_ids)
            loss = policy_loss + args.value_coef * value_loss - entropy_coef * entropy + args.aux_coef * aux_loss
            optimizer.zero_grad()
            loss.backward()
            stats["grad_norm"].append(clip_gradients(optimizer.parameters, args.max_grad_norm))
            optimizer.step()
            with torch.no_grad():
                stats["kl"].append(float(((ratio - 1) - (new_logp - old_logp)).mean().cpu()))
                stats["clipfrac"].append(float(((ratio - 1).abs() > args.clip).float().mean().cpu()))
            for key, item in (("policy_loss", policy_loss), ("value_loss", value_loss), ("entropy", entropy),
                              ("aux_loss", aux_loss)):
                stats[key].append(float(item.detach().cpu()))
    if not np.isfinite(stats["policy_loss"]).all():
        raise RuntimeError("Non-finite PPO loss")
    values, rets = data["value"].reshape(-1)[selected], returns_all[selected]
    summary = {k: round(float(np.mean(v)), 4) for k, v in stats.items()}
    summary["explained_variance"] = round(float(1 - np.var(rets - values) / (np.var(rets) + 1e-8)), 3)
    return summary, len(selected)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--minutes", type=float, default=60.0, help="training wall-clock budget")
    parser.add_argument("--device", choices=["auto", "directml", "cuda", "cpu"], default="auto")
    parser.add_argument("--adapter", type=int, default=0)
    parser.add_argument("--pool", type=Path, default=REPO / "data/teams/train-s42-2000.txt")
    parser.add_argument("--envs", type=int, default=128)
    parser.add_argument("--workers", type=int, default=4, help="actor processes (the learner is the bottleneck; leave cores for it)")
    parser.add_argument("--steps", type=int, default=32, help="decision boundaries per rollout")
    parser.add_argument("--epochs", type=int, default=2)
    parser.add_argument("--minibatch", type=int, default=1024)
    parser.add_argument("--width", type=int, default=128)
    parser.add_argument("--layers", type=int, default=3)
    parser.add_argument("--lr", type=float, default=3e-4)
    parser.add_argument("--lr-final", type=float, default=0.3, help="final learning-rate multiplier")
    parser.add_argument("--gamma", type=float, default=0.995)
    parser.add_argument("--lam", type=float, default=0.95)
    parser.add_argument("--clip", type=float, default=0.2)
    parser.add_argument("--entropy", type=float, default=0.01)
    parser.add_argument("--entropy-final", type=float, default=0.003)
    parser.add_argument("--value-coef", type=float, default=0.5)
    parser.add_argument("--aux-coef", type=float, default=0.03)
    parser.add_argument("--max-grad-norm", type=float, default=0.5)
    parser.add_argument("--ko-bonus", type=float, default=0.05, help="per KO difference; decays to 0 at half the run")
    parser.add_argument("--pool-fraction", type=float, default=0.2)
    parser.add_argument("--pool-minutes", type=float, default=3.0)
    parser.add_argument("--pool-size", type=int, default=12)
    parser.add_argument("--snapshot-minutes", type=float, default=5.0)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--output", type=Path, default=REPO / "scratch/training/selfplay")
    parser.add_argument("--resume", type=Path)
    args = parser.parse_args()

    torch.manual_seed(args.seed)
    torch.set_num_threads(1)
    rng = np.random.default_rng(args.seed)
    device, adapter = training_device(args.device, args.adapter)
    dex = load_dex()
    model = Model(dex, args.width, args.layers).to(device)
    optimizer = Adam(model.parameters(), lr=args.lr)
    pool: list[dict] = []
    elapsed_before = 0.0
    update = battles = samples_total = 0
    if args.resume:
        state = torch.load(args.resume, map_location="cpu", weights_only=False)
        model.load_state_dict(state["model"])
        optimizer.load_state_dict(state["optimizer"])
        pool = state["pool"]
        elapsed_before, update, battles, samples_total = state["elapsed"], state["update"], state["battles"], state["samples"]
        rng.bit_generator.state = state["numpy_rng"]
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "config.json").write_text(json.dumps({k: str(v) for k, v in vars(args).items()}, indent=1))
    n_params = sum(p.numel() for p in model.parameters())
    print(json.dumps({"adapter": adapter, "device": str(device), "parameters": n_params}), flush=True)

    pool_envs = spread_mask(args.envs, args.pool_fraction)
    budget = args.minutes * 60
    next_pool = elapsed_before + args.pool_minutes * 60 if pool else 0.0
    next_snapshot = (int(elapsed_before // (args.snapshot_minutes * 60)) + (1 if elapsed_before else 0)) * args.snapshot_minutes * 60

    def checkpoint(elapsed):
        save(args.output / "latest.pt", {
            "model": cpu_state(model), "optimizer": optimizer.state_dict(), "pool": pool, "elapsed": elapsed,
            "update": update, "battles": battles, "samples": samples_total, "numpy_rng": rng.bit_generator.state,
            "width": args.width, "layers": args.layers})

    def launch(actors, buffer: int, elapsed: float) -> dict:
        """Hand the idle actors the current weights and an opponent, and start filling ``buffer``."""
        nonlocal pool, next_pool
        if elapsed >= next_pool:
            pool.append(cpu_state(model))
            pool = pool[-args.pool_size:]
            next_pool = elapsed + args.pool_minutes * 60
        progress = min(elapsed / budget, 1.0)
        use_pool = len(pool) > 1
        actors.set_weights("learner", cpu_state(model))
        if use_pool:
            actors.set_weights("opponent", pool[int(rng.integers(len(pool) - 1))])
        ko_bonus = args.ko_bonus * max(0.0, 1 - 2 * progress)
        actors.start(buffer, use_pool, pool_envs, ko_bonus)
        return dict(use_pool=use_pool, ko_bonus=ko_bonus)

    with Actors(args.envs, args.steps, [args.pool], args.seed + update, args.workers, model) as actors:
        start = time.perf_counter() - elapsed_before
        current = 0
        info = launch(actors, current, elapsed_before)
        rollout = actors.wait()
        while True:
            elapsed = time.perf_counter() - start
            if elapsed >= next_snapshot:
                save(args.output / "snapshots" / f"m{int(round(next_snapshot / 60)):03d}.pt",
                     {"model": cpu_state(model), "width": args.width, "layers": args.layers, "minutes": elapsed / 60,
                      "battles": battles, "update": update})
                next_snapshot += args.snapshot_minutes * 60
            if elapsed >= budget:
                break
            # The actors fill the other buffer with the current weights while the GPU trains on this one.
            collected, collected_info = rollout, info
            info = launch(actors, 1 - current, elapsed)
            progress = min(elapsed / budget, 1.0)
            optimizer.lr = args.lr * (1 - (1 - args.lr_final) * progress)
            entropy_coef = args.entropy + (args.entropy_final - args.entropy) * progress
            t1 = time.perf_counter()
            model.train()
            stats, samples = ppo_update(model, optimizer, actors.buffers[current].arrays, args, rng, entropy_coef, device)
            train_seconds = time.perf_counter() - t1
            rollout = actors.wait()
            cycle = time.perf_counter() - t1
            current = 1 - current
            update += 1
            battles += collected["completed"]
            samples_total += samples
            metrics = {
                "update": update, "minutes": round((time.perf_counter() - start) / 60, 3), "battles": battles,
                "samples": samples_total, "battles_per_s": round(collected["completed"] / cycle, 1),
                "cycle_s": round(cycle, 2), "train_s": round(train_seconds, 2), "wait_s": round(cycle - train_seconds, 2),
                "rollout_s": round(collected["seconds"], 2), "actor_inference_s": round(collected["inference_seconds"], 2),
                "mean_turns": round(collected["turns"] / max(collected["completed"], 1), 1),
                "pool_games": collected["pool_games"],
                "pool_win_rate": round(collected["pool_wins"] / collected["pool_games"], 3) if collected["pool_games"] else None,
                "lr": optimizer.lr, "entropy_coef": round(entropy_coef, 5), "ko_bonus": round(collected_info["ko_bonus"], 4),
                "pool_size": len(pool), **stats, **memory_mb(),
                **{k: v for k, v in collected.items() if k.startswith("actor_")},
            }
            print(json.dumps(metrics), flush=True)
            with (args.output / "metrics.jsonl").open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(metrics) + "\n")
            if update % 10 == 0:
                checkpoint(time.perf_counter() - start)
    checkpoint(time.perf_counter() - start)
    print(f"Done: {update} updates, {battles} battles in {(time.perf_counter() - start) / 60:.1f} min", flush=True)


if __name__ == "__main__":
    main()
