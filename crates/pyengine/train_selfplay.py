"""Time-budgeted PPO self-play with the transformer policy (docs/training/TIPS.md, first run).

Run from the repository root:

    .venv\\Scripts\\python.exe crates\\pyengine\\train_selfplay.py --minutes 60

* Observations: training.encoder (own request + player-view protocol only), damage features (training.damage).
* Model: training.model (token transformer, per-action scoring, slot b conditioned on slot a, oracle critic,
  auxiliary hidden-information heads).
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

from training.device import training_device
from training.dex import load_dex
from training.model import Model
from training.ppo import PASS, Adam, clip_gradients, gae, sample
from training.vecenv import VecEnv

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


class Rollout:
    """[T, rows] storage for one rollout."""

    def __init__(self, steps: int, rows: int, arrays: dict):
        self.obs = {k: np.zeros((steps, *arrays[k].shape), dtype=arrays[k].dtype) for k in OBS_KEYS}
        shape = (steps, rows)
        self.mask0 = np.zeros((*shape, N_ACTIONS), dtype=bool)
        self.mask1 = np.zeros((*shape, N_ACTIONS), dtype=bool)
        self.a0 = np.zeros(shape, dtype=np.int64)
        self.a1 = np.zeros(shape, dtype=np.int64)
        self.logp = np.zeros(shape, dtype=np.float32)
        self.value = np.zeros(shape, dtype=np.float32)
        self.reward = np.zeros(shape, dtype=np.float32)
        self.done = np.zeros(shape, dtype=bool)
        self.train = np.zeros(shape, dtype=bool)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--minutes", type=float, default=60.0, help="training wall-clock budget")
    parser.add_argument("--device", choices=["auto", "directml", "cuda", "cpu"], default="auto")
    parser.add_argument("--adapter", type=int, default=0)
    parser.add_argument("--pool", type=Path, default=REPO / "data/teams/train-s42-2000.txt")
    parser.add_argument("--envs", type=int, default=128)
    parser.add_argument("--workers", type=int, default=4)
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
    torch.set_num_threads(2)
    rng = np.random.default_rng(args.seed)
    device, adapter = training_device(args.device, args.adapter)
    dex = load_dex()
    model = Model(dex, args.width, args.layers).to(device)
    opponent = Model(dex, args.width, args.layers).to(device)
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

    E, R = args.envs, 2 * args.envs
    partner = np.arange(R) ^ 1
    pool_envs = np.arange(E) < int(round(args.pool_fraction * E))
    opponent_rows = np.zeros(R, dtype=bool)
    opponent_rows[1::2] = pool_envs  # side 2 of the pool battles
    budget = args.minutes * 60
    next_pool = elapsed_before + args.pool_minutes * 60 if pool else 0.0
    next_snapshot = (int(elapsed_before // (args.snapshot_minutes * 60)) + (1 if elapsed_before else 0)) * args.snapshot_minutes * 60

    with VecEnv(E, [args.pool], seed=args.seed + update, workers=args.workers) as env:
        obs = env.reset()
        rollout = Rollout(args.steps, R, obs)
        start = time.perf_counter() - elapsed_before
        while True:
            elapsed = time.perf_counter() - start
            if elapsed >= next_snapshot:
                save(args.output / "snapshots" / f"m{int(round(next_snapshot / 60)):03d}.pt",
                     {"model": cpu_state(model), "width": args.width, "layers": args.layers, "minutes": elapsed / 60,
                      "battles": battles, "update": update})
                next_snapshot += args.snapshot_minutes * 60
            if elapsed >= budget:
                break
            if elapsed >= next_pool:
                pool.append(cpu_state(model))
                pool = pool[-args.pool_size:]
                next_pool = elapsed + args.pool_minutes * 60
            progress = min(elapsed / budget, 1.0)
            lr = args.lr * (1 - (1 - args.lr_final) * progress)
            optimizer.lr = lr
            entropy_coef = args.entropy + (args.entropy_final - args.entropy) * progress
            ko_bonus = args.ko_bonus * max(0.0, 1 - 2 * progress)
            # Opponent for this rollout: a random older snapshot (learner self-play until the pool has one).
            use_pool = len(pool) > 1
            if use_pool:
                opponent.load_state_dict(pool[int(rng.integers(len(pool) - 1))])
            opp_rows = opponent_rows if use_pool else np.zeros(R, dtype=bool)

            # ---- rollout ----------------------------------------------------------------------------------------
            t0 = time.perf_counter()
            model.eval()
            completed = turns = pool_games = pool_wins = 0
            for t in range(args.steps):
                for k in OBS_KEYS:
                    rollout.obs[k][t] = obs[k]
                active = obs["needs_action"].reshape(R).copy()
                fainted_before = obs["fainted"].copy()
                mask0 = obs["mask0"].reshape(R, N_ACTIONS).copy()
                mask0[~active] = False
                mask0[~active, PASS] = True
                with torch.no_grad():
                    batch = upload(obs, slice(None), device)
                    state = model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"], batch["token_features"])
                    value = model.value(state, *partner_view(obs, partner, device))
                    lp0_t, vectors0 = model.slot0(state, torch.from_numpy(mask0).to(device))
                    lp0 = lp0_t.cpu().numpy()
                    if use_pool:
                        rows = np.flatnonzero(opp_rows)
                        sub = upload(obs, rows, device)
                        opp_state = opponent.trunk(sub["ids"], sub["floats"], sub["field"], sub["action_features"], sub["token_features"])
                        opp_lp0_t, opp_vectors0 = opponent.slot0(opp_state, torch.from_numpy(mask0[rows]).to(device))
                        lp0[rows] = opp_lp0_t.cpu().numpy()
                    a0 = sample(lp0, rng)
                    requested = np.where(active, a0, -1).reshape(E, 2)
                    mask1 = env.mask_slot1(requested).reshape(R, N_ACTIONS).copy()
                    mask1[~active] = False
                    mask1[~active, PASS] = True
                    a0_t = torch.from_numpy(a0.astype(np.int64)).to(device)
                    lp1 = model.slot1(state, vectors0, a0_t, torch.from_numpy(mask1).to(device)).cpu().numpy()
                    if use_pool:
                        opp_lp1 = opponent.slot1(opp_state, opp_vectors0, torch.from_numpy(a0[rows].astype(np.int64)).to(device),
                                                 torch.from_numpy(mask1[rows]).to(device)).cpu().numpy()
                        lp1[rows] = opp_lp1
                    a1 = sample(lp1, rng)
                    value = value.cpu().numpy()
                actions = np.stack((a0, a1), -1).reshape(E, 2, 2)
                actions[~obs["needs_action"]] = -1
                rollout.mask0[t], rollout.mask1[t] = mask0, mask1
                rollout.a0[t], rollout.a1[t] = a0, a1
                rollout.logp[t] = lp0[np.arange(R), a0] + lp1[np.arange(R), a1]
                rollout.value[t] = value
                rollout.train[t] = active & ~opp_rows
                obs = env.step(actions)
                done = obs["done"]
                reward = obs["reward"].reshape(R).copy()
                if ko_bonus > 0:
                    delta = obs["fainted"] - fainted_before  # own, foe fainted counts per row
                    shaped = ko_bonus * (delta[:, 1] - delta[:, 0])
                    reward += np.where(np.repeat(done, 2), 0.0, shaped)
                rollout.reward[t] = reward
                rollout.done[t] = np.repeat(done, 2)
                completed += int(done.sum())
                turns += int(obs["final_turns"][done].sum())
                if use_pool:
                    finished = done & pool_envs
                    pool_games += int(finished.sum())
                    pool_wins += int((obs["winner"][finished] == 0).sum())
            with torch.no_grad():
                batch = upload(obs, slice(None), device)
                state = model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"], batch["token_features"])
                bootstrap = model.value(state, *partner_view(obs, partner, device)).cpu().numpy()
            rollout_seconds = time.perf_counter() - t0

            # ---- PPO update -------------------------------------------------------------------------------------
            t1 = time.perf_counter()
            model.train()
            advantages, returns = gae(rollout.reward, rollout.value, rollout.done, bootstrap, args.gamma, args.lam)
            selected = np.flatnonzero(rollout.train.reshape(-1))
            flat_obs = {k: v.reshape(-1, *v.shape[2:]) for k, v in rollout.obs.items()}
            partner_index = (np.arange(args.steps)[:, None] * R + partner[None, :]).reshape(-1)
            adv = advantages.reshape(-1)[selected]
            adv = (adv - adv.mean()) / (adv.std() + 1e-8)
            stats = {k: [] for k in ("policy_loss", "value_loss", "entropy", "aux_loss", "kl", "clipfrac", "grad_norm")}
            for _ in range(args.epochs):
                order = rng.permutation(len(selected))
                for begin in range(0, len(order), args.minibatch):
                    pick = order[begin:begin + args.minibatch]
                    if len(pick) < args.minibatch // 4:
                        continue
                    index = selected[pick]
                    batch = upload(flat_obs, index, device)
                    partner_ids, partner_floats = partner_view(flat_obs, partner_index[index], device)
                    a0 = torch.from_numpy(rollout.a0.reshape(-1)[index]).to(device)
                    a1 = torch.from_numpy(rollout.a1.reshape(-1)[index]).to(device)
                    mask0 = torch.from_numpy(rollout.mask0.reshape(-1, N_ACTIONS)[index]).to(device)
                    mask1 = torch.from_numpy(rollout.mask1.reshape(-1, N_ACTIONS)[index]).to(device)
                    state = model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"], batch["token_features"])
                    lp0, vectors0 = model.slot0(state, mask0)
                    lp1 = model.slot1(state, vectors0, a0, mask1)
                    new_logp = (lp0 * onehot(a0)).sum(-1) + (lp1 * onehot(a1)).sum(-1)
                    old_logp = torch.from_numpy(rollout.logp.reshape(-1)[index]).to(device)
                    ratio = (new_logp - old_logp).exp()
                    weight = torch.from_numpy(adv[pick].astype(np.float32)).to(device)
                    policy_loss = -torch.minimum(ratio * weight, ratio.clamp(1 - args.clip, 1 + args.clip) * weight).mean()
                    value = model.value(state, partner_ids, partner_floats)
                    target = torch.from_numpy(returns.reshape(-1)[index].astype(np.float32)).to(device)
                    value_loss = (value - target).square().mean()
                    entropy = -((lp0.exp() * lp0).sum(-1) + (lp1.exp() * lp1).sum(-1)).mean()
                    aux_loss = model.aux_loss(state, batch["floats"], batch["foe_match"], partner_ids)
                    loss = policy_loss + args.value_coef * value_loss - entropy_coef * entropy + args.aux_coef * aux_loss
                    optimizer.zero_grad()
                    loss.backward()
                    stats["grad_norm"].append(clip_gradients(optimizer.parameters, args.max_grad_norm))
                    optimizer.step()
                    with torch.no_grad():
                        log_ratio = new_logp - old_logp
                        stats["kl"].append(float(((ratio - 1) - log_ratio).mean().cpu()))
                        stats["clipfrac"].append(float(((ratio - 1).abs() > args.clip).float().mean().cpu()))
                    for key, item in (("policy_loss", policy_loss), ("value_loss", value_loss), ("entropy", entropy),
                                      ("aux_loss", aux_loss)):
                        stats[key].append(float(item.detach().cpu()))
            if not np.isfinite(stats["policy_loss"]).all():
                raise RuntimeError("Non-finite PPO loss")
            train_seconds = time.perf_counter() - t1
            update += 1
            battles += completed
            samples_total += len(selected)
            values, rets = rollout.value.reshape(-1)[selected], returns.reshape(-1)[selected]
            metrics = {
                "update": update, "minutes": round((time.perf_counter() - start) / 60, 3), "battles": battles,
                "samples": samples_total, "battles_per_s": round(completed / (rollout_seconds + train_seconds), 1),
                "rollout_s": round(rollout_seconds, 2), "train_s": round(train_seconds, 2),
                "mean_turns": round(turns / max(completed, 1), 1), "pool_games": pool_games,
                "pool_win_rate": round(pool_wins / pool_games, 3) if pool_games else None,
                "explained_variance": round(float(1 - np.var(rets - values) / (np.var(rets) + 1e-8)), 3),
                "lr": lr, "entropy_coef": round(entropy_coef, 5), "ko_bonus": round(ko_bonus, 4), "pool_size": len(pool),
                **{k: round(float(np.mean(v)), 4) for k, v in stats.items()},
            }
            print(json.dumps(metrics), flush=True)
            with (args.output / "metrics.jsonl").open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(metrics) + "\n")
            if update % 10 == 0:
                save(args.output / "latest.pt", {
                    "model": cpu_state(model), "optimizer": optimizer.state_dict(), "pool": pool,
                    "elapsed": time.perf_counter() - start, "update": update, "battles": battles,
                    "samples": samples_total, "numpy_rng": rng.bit_generator.state, "width": args.width,
                    "layers": args.layers})
    save(args.output / "latest.pt", {
        "model": cpu_state(model), "optimizer": optimizer.state_dict(), "pool": pool,
        "elapsed": time.perf_counter() - start, "update": update, "battles": battles, "samples": samples_total,
        "numpy_rng": rng.bit_generator.state, "width": args.width, "layers": args.layers})
    print(f"Done: {update} updates, {battles} battles in {(time.perf_counter() - start) / 60:.1f} min", flush=True)


if __name__ == "__main__":
    main()
