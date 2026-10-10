"""Time-budgeted PPO self-play with the transformer policy (docs/training/TIPS.md, docs/training/SELFPLAY.md).

Run from the repository root:

    .venv\\Scripts\\python.exe crates\\pyengine\\train_selfplay.py --minutes 60

* Observations: training.encoder (own request + player-view protocol only), damage features (training.damage).
* Model: training.model (token transformer, per-action scoring, slot b conditioned on slot a, oracle critic,
  auxiliary hidden-information heads).
* Rollouts: actor processes (training.actors) play the battles with CPU copies of the policy and fill one of two
  shared-memory buffers while the GPU trains on the other, so each rollout is collected with the weights from one
  update earlier. PPO's ratio uses the stored behaviour log-probs.
* Opponents (training.league): the learner plays both sides of most battles; ``--pool-fraction`` of the battles
  (fixed envs) put another player on side 2: ``--scripted-share`` of them the "always use the strongest attack"
  heuristic, the rest an older snapshot (added every ``--pool-minutes``) chosen per rollout by prioritized
  fictitious self-play from the learner's results against each snapshot. Only the learner's own decisions are
  trained on.
* Reward: +1 win, -1 loss, plus a potential-based KO-difference bonus (sums to zero over a battle) that decays
  linearly to zero at ``--ko-bonus-end`` of the run.
* Exploration: the entropy-bonus coefficient adapts so the summed slot-a + slot-b policy entropy tracks a target
  that falls linearly from ``--entropy-target`` to ``--entropy-target-final`` (``--entropy-target 0``: the fixed
  ``--entropy`` -> ``--entropy-final`` schedule instead).
* Schedule: the learning rate anneals over the time budget; snapshots every ``--snapshot-minutes`` feed
  evaluate.py. ``latest.pt`` (with optimizer, opponent-pool and entropy-controller state) supports ``--resume``.
* Metrics: besides the losses, win rates against snapshots and against the heuristic, the PFSP table, and the
  learner's action usage (Tera, switches, Protect, status moves, Fake Out; training.diagnostics).
"""
from __future__ import annotations

import argparse
import ctypes
import json
import os
import sys
import time
from pathlib import Path

import numpy as np
import torch

from training.actors import Actors
from training.device import training_device
from training.diagnostics import ActionUsage
from training.dex import load_dex
from training.league import HEURISTIC, SELF, SNAPSHOT, SnapshotPool, opponent_kinds
from training.model import Model
from training.ppo import Adam, EntropyController, clip_gradients, gae

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


def memory_mb() -> dict:
    """Working set and private bytes of this process (Windows), for leak checks."""
    if sys.platform != "win32":
        return {}

    class Counters(ctypes.Structure):
        _fields_ = [("cb", ctypes.c_ulong), ("PageFaultCount", ctypes.c_ulong)] + [
            (name, ctypes.c_size_t) for name in ("PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
                                                 "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
                                                 "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage",
                                                 "PrivateUsage")]

    counters = Counters()
    counters.cb = ctypes.sizeof(counters)
    process = ctypes.windll.kernel32.GetCurrentProcess()
    if not ctypes.windll.psapi.GetProcessMemoryInfo(process, ctypes.byref(counters), counters.cb):
        return {}
    return {"rss_mb": counters.WorkingSetSize >> 20, "private_mb": counters.PrivateUsage >> 20}


def opponent_results(collected: dict, info: dict) -> dict:
    """Finished games, the learner's score (not for self-play) and mean battle turns by opponent kind for one
    rollout, over all games that finished in the envs of that kind."""
    out = {}
    for kind, name in ((SELF, "self"), (SNAPSHOT, "pool"), (HEURISTIC, "scripted")):
        envs = info["opponents"] == kind
        games = float(collected["games"][envs].sum())
        out[f"{name}_games"] = int(games)
        if kind != SELF:
            out[f"{name}_win_rate"] = round(float(collected["score"][envs].sum()) / games, 3) if games else None
        out[f"{name}_turns"] = round(float(collected["battle_turns"][envs].sum()) / games, 1) if games else None
    return out


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
    parser.add_argument("--workers", type=int, default=6, help="actor processes (leave a core for the learner)")
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
    parser.add_argument("--entropy", type=float, default=0.01,
                        help="entropy-bonus coefficient (the starting value of the adaptive coefficient)")
    parser.add_argument("--entropy-final", type=float, default=0.003,
                        help="final coefficient of the fixed linear schedule (only with --entropy-target 0)")
    parser.add_argument("--entropy-target", type=float, default=2.5,
                        help="target for the summed slot-a + slot-b entropy at the start (0: fixed schedule)")
    parser.add_argument("--entropy-target-final", type=float, default=1.5, help="entropy target at the end of the run")
    parser.add_argument("--entropy-rate", type=float, default=0.2,
                        help="change of log(coefficient) per update and nat of entropy error")
    parser.add_argument("--entropy-coef-min", type=float, default=0.001)
    parser.add_argument("--entropy-coef-max", type=float, default=0.05)
    parser.add_argument("--value-coef", type=float, default=0.5)
    parser.add_argument("--aux-coef", type=float, default=0.03)
    parser.add_argument("--max-grad-norm", type=float, default=0.5)
    parser.add_argument("--ko-bonus", type=float, default=0.02,
                        help="potential-based bonus per KO difference (sums to zero over a battle)")
    parser.add_argument("--ko-bonus-end", type=float, default=0.25,
                        help="fraction of the run at which the KO bonus has decayed linearly to zero")
    parser.add_argument("--pool-fraction", type=float, default=0.3,
                        help="share of battles against a snapshot or the heuristic instead of pure self-play")
    parser.add_argument("--scripted-share", type=float, default=0.33,
                        help="share of those battles against the strongest-attack heuristic")
    parser.add_argument("--pool-minutes", type=float, default=3.0)
    parser.add_argument("--pool-size", type=int, default=12)
    parser.add_argument("--pfsp-power", type=float, default=2.0,
                        help="PFSP: snapshot weight (1 - learner score EMA) ** power")
    parser.add_argument("--pfsp-floor", type=float, default=0.1, help="PFSP: uniform share mixed into the weights")
    parser.add_argument("--pfsp-alpha", type=float, default=0.02, help="PFSP: per-game EMA rate of the learner score")
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
    pool = SnapshotPool(args.pool_size, args.pfsp_power, args.pfsp_floor, args.pfsp_alpha)
    controller = (EntropyController(args.entropy, args.entropy_target, args.entropy_target_final, args.entropy_rate,
                                    args.entropy_coef_min, args.entropy_coef_max) if args.entropy_target > 0 else None)
    usage = ActionUsage(dex)
    elapsed_before = 0.0
    update = battles = samples_total = 0
    if args.resume:
        state = torch.load(args.resume, map_location="cpu", weights_only=False)
        model.load_state_dict(state["model"])
        optimizer.load_state_dict(state["optimizer"])
        pool.load_state_dict(state["pool"])
        if controller is not None and state.get("entropy_controller"):
            controller.load_state_dict(state["entropy_controller"])
        elapsed_before, update, battles, samples_total = state["elapsed"], state["update"], state["battles"], state["samples"]
        rng.bit_generator.state = state["numpy_rng"]
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "config.json").write_text(json.dumps({k: str(v) for k, v in vars(args).items()}, indent=1))
    n_params = sum(p.numel() for p in model.parameters())
    print(json.dumps({"adapter": adapter, "device": str(device), "parameters": n_params}), flush=True)

    kinds = opponent_kinds(args.envs, args.pool_fraction, args.scripted_share)  # fixed per env for the whole run
    budget = args.minutes * 60
    next_pool = elapsed_before + args.pool_minutes * 60 if len(pool) else 0.0
    next_snapshot = (int(elapsed_before // (args.snapshot_minutes * 60)) + (1 if elapsed_before else 0)) * args.snapshot_minutes * 60
    loaded = {"snapshot": None}  # id of the snapshot in the actors' opponent weights

    def checkpoint(elapsed):
        save(args.output / "latest.pt", {
            "model": cpu_state(model), "optimizer": optimizer.state_dict(), "pool": pool.state_dict(),
            "entropy_controller": controller.state_dict() if controller is not None else None, "elapsed": elapsed,
            "update": update, "battles": battles, "samples": samples_total, "numpy_rng": rng.bit_generator.state,
            "width": args.width, "layers": args.layers})

    def launch(actors, buffer: int, elapsed: float) -> dict:
        """Hand the idle actors the current weights and opponents, and start filling ``buffer``."""
        nonlocal next_pool
        if elapsed >= next_pool:
            pool.add(cpu_state(model), elapsed / 60)
            next_pool = elapsed + args.pool_minutes * 60
        progress = min(elapsed / budget, 1.0)
        actors.set_weights("learner", cpu_state(model))
        snapshot = pool.sample(rng)  # PFSP; None until there are two snapshots
        if snapshot is not None and snapshot["id"] != loaded["snapshot"]:
            actors.set_weights("opponent", snapshot["state"])
            loaded["snapshot"] = snapshot["id"]
        opponents = np.where((kinds == SNAPSHOT) & (snapshot is None), SELF, kinds).astype(np.int8)
        ko_bonus = args.ko_bonus * max(0.0, 1 - progress / args.ko_bonus_end) if args.ko_bonus_end > 0 else 0.0
        actors.start(buffer, opponents, ko_bonus)
        return dict(opponents=opponents, ko_bonus=ko_bonus, snapshot=None if snapshot is None else snapshot["id"],
                    snapshot_minutes=None if snapshot is None else snapshot["minutes"])

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
            snapshot_envs = collected_info["opponents"] == SNAPSHOT  # PFSP: credit games begun against the snapshot
            pool.record(collected_info["snapshot"], float(collected["clean_games"][snapshot_envs].sum()),
                        float(collected["clean_score"][snapshot_envs].sum()))
            info = launch(actors, 1 - current, elapsed)
            progress = min(elapsed / budget, 1.0)
            optimizer.lr = args.lr * (1 - (1 - args.lr_final) * progress)
            if controller is not None:
                entropy_coef = controller.coef
            else:
                entropy_coef = args.entropy + (args.entropy_final - args.entropy) * progress
            t1 = time.perf_counter()
            model.train()
            stats, samples = ppo_update(model, optimizer, actors.buffers[current].arrays, args, rng, entropy_coef, device)
            train_seconds = time.perf_counter() - t1
            if controller is not None:
                controller.update(stats["entropy"], progress)
            behaviour = usage(actors.buffers[current].arrays)  # the actors are filling the other buffer
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
                **opponent_results(collected, collected_info), "opponent_minutes": collected_info["snapshot_minutes"],
                "lr": optimizer.lr, "entropy_coef": round(entropy_coef, 5),
                "entropy_target": round(controller.target(progress), 3) if controller is not None else None,
                "ko_bonus": round(collected_info["ko_bonus"], 4), "pool_size": len(pool), **stats, **behaviour,
                "pfsp": pool.summary(), **memory_mb(),
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
