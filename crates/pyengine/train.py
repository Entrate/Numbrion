"""PPO self-play baseline with autoregressive masks, public observations and checkpoint resume.

Run from the repository root: .venv/Scripts/python crates/pyengine/train.py --device directml
This small MLP is a starting pipeline, not a competitive trained Pokemon agent.
"""
from __future__ import annotations

import argparse
import json
import os
import time
from pathlib import Path

import numpy as np
import torch

import numbrion as nb
from training.device import training_device
from training.observations import PublicEncoder
from training.policy import N_ACTIONS, Policy, masked_logprobs

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]


def advantages(rewards, values, done, bootstrap, gamma=0.99, gae_lambda=0.95):
    output = np.zeros_like(rewards)
    carry = np.zeros_like(bootstrap)
    for t in reversed(range(len(rewards))):
        following = bootstrap if t == len(rewards) - 1 else values[t + 1]
        continuation = 1.0 - done[t].astype(np.float32)
        delta = rewards[t] + gamma * following * continuation - values[t]
        carry = delta + gamma * gae_lambda * continuation * carry
        output[t] = carry
    return output, output + values


def tensor(array, device):
    return torch.from_numpy(np.asarray(array, dtype=np.float32)).to(device)


def onehot(actions):
    return np.eye(N_ACTIONS, dtype=np.float32)[actions]


def sample(logprobs, rng):
    probabilities = logprobs.detach().cpu().numpy()
    probabilities = np.exp(probabilities).astype(np.float64)
    probabilities /= probabilities.sum(axis=-1, keepdims=True)
    cdf = probabilities.cumsum(axis=-1)
    cdf[:, -1] = 1.0
    return np.minimum((cdf < rng.random((len(cdf), 1))).sum(axis=-1), N_ACTIONS - 1).astype(np.int32)


def clip_gradients(model, max_norm=0.5):
    # Scalar reduction and scaling avoid unsupported DirectML foreach/linalg kernels.
    squared = sum(p.grad.detach().square().sum() for p in model.parameters() if p.grad is not None)
    norm = float(squared.sqrt().cpu())
    if not np.isfinite(norm):
        raise RuntimeError("Non-finite gradient norm")
    if norm > max_norm:
        for p in model.parameters():
            if p.grad is not None:
                p.grad.mul_(max_norm / (norm + 1e-6))
    return norm


def save_checkpoint(path, model, optimizer, args, update, battles, rng):
    def cpu_state(value):
        if isinstance(value, torch.Tensor):
            return value.detach().cpu()
        if isinstance(value, dict):
            return {key: cpu_state(item) for key, item in value.items()}
        if isinstance(value, list):
            return [cpu_state(item) for item in value]
        return value

    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    torch.save({
        "format": 1, "model": {k: v.detach().cpu() for k, v in model.state_dict().items()},
        "optimizer": cpu_state(optimizer.state_dict()), "config": vars(args), "update": update,
        "battles": battles, "numpy_rng": rng.bit_generator.state, "torch_rng": torch.get_rng_state(),
    }, temporary)
    os.replace(temporary, path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--device", choices=["auto", "directml", "cuda", "cpu"], default="auto")
    parser.add_argument("--adapter", type=int, default=0)
    parser.add_argument("--pool", type=Path, default=REPO / "data/teams/train-s42-2000.txt")
    parser.add_argument("--envs", type=int, default=32)
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--steps", type=int, default=64, help="decision boundaries per rollout")
    parser.add_argument("--updates", type=int, default=100, help="additional PPO updates, including on resume")
    parser.add_argument("--epochs", type=int, default=4)
    parser.add_argument("--batch-size", type=int, default=256)
    parser.add_argument("--width", type=int, default=128)
    parser.add_argument("--lr", type=float, default=0.0003)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--output", type=Path, default=REPO / "scratch/training/directml")
    parser.add_argument("--resume", type=Path)
    args = parser.parse_args()
    if min(args.envs, args.threads, args.steps, args.updates, args.epochs, args.batch_size, args.width) < 1 or args.lr <= 0:
        parser.error("Training counts and learning rate must be positive")
    if not args.pool.is_file():
        parser.error(f"Team pool missing: {args.pool}; see docs/training/WINDOWS-GPU.md")
    torch.set_num_threads(2)
    torch.manual_seed(args.seed)
    rng = np.random.default_rng(args.seed)
    device, adapter_name = training_device(args.device, args.adapter)
    model = Policy(args.width).to(device)
    # RMSprop avoids Adam's unsupported DirectML lerp kernel and its CPU fallback.
    optimizer = torch.optim.RMSprop(model.parameters(), lr=args.lr, eps=1e-5, foreach=False)
    first_update = battles = 0
    if args.resume:
        state = torch.load(args.resume, map_location="cpu", weights_only=False)
        if state.get("format") != 1 or state["config"]["width"] != args.width:
            raise ValueError("Checkpoint format/width mismatch")
        model.load_state_dict(state["model"])
        optimizer.load_state_dict(state["optimizer"])
        first_update, battles = state["update"], state["battles"]
        rng.bit_generator.state = state["numpy_rng"]
        torch.set_rng_state(state["torch_rng"])
    initial_weights = [p.detach().cpu().clone() for p in model.parameters()]
    env = nb.BatchEnv(args.envs, [args.pool], seed=args.seed + first_update, threads=args.threads, log=True)
    encoder = PublicEncoder(args.envs)
    result = env.reset()
    rows = args.envs * 2

    def observe():
        return np.stack([encoder.encode(env.observe(e, s), e * 2 + s, s) for e in range(args.envs) for s in (0, 1)])

    obs = observe()
    args.output.mkdir(parents=True, exist_ok=True)
    print(json.dumps({"adapter": adapter_name, "device": str(device), "parameters": sum(p.numel() for p in model.parameters()), "envs": args.envs, "threads": args.threads}), flush=True)
    start = time.perf_counter()
    for update in range(first_update + 1, first_update + args.updates + 1):
        rollout = {key: [] for key in ("obs", "mask0", "mask1", "a0", "a1", "logp", "value", "reward", "done", "active")}
        completed = 0
        for _ in range(args.steps):
            active = result["needs_action"].reshape(rows)
            mask0 = result["mask0"].reshape(rows, N_ACTIONS).copy()
            mask0[~active, nb.PASS] = True
            with torch.no_grad():
                hidden, logits0, value = model(tensor(obs, device))
                lp0 = masked_logprobs(logits0, tensor(mask0, device))
                a0 = sample(lp0, rng)
                requested_a0 = np.where(active, a0, -1).reshape(args.envs, 2).astype(np.int32)
                mask1 = env.mask_slot1(requested_a0).reshape(rows, N_ACTIONS).copy()
                mask1[~active, nb.PASS] = True
                lp1 = masked_logprobs(model.second(hidden, tensor(onehot(a0), device)), tensor(mask1, device))
                a1 = sample(lp1, rng)
                logp = (lp0 * tensor(onehot(a0), device)).sum(-1) + (lp1 * tensor(onehot(a1), device)).sum(-1)
            actions = np.stack((a0, a1), axis=-1).reshape(args.envs, 2, 2)
            actions[~result["needs_action"]] = -1
            following = env.step(actions.astype(np.int32))
            if following["illegal"].any():
                raise RuntimeError("A sampled action violated the exact mask")
            for key, data in {
                "obs": obs, "mask0": mask0, "mask1": mask1, "a0": a0, "a1": a1,
                "logp": logp.cpu().numpy(), "value": value.cpu().numpy(),
                "reward": following["reward"].reshape(rows),
                "done": np.repeat(following["done"], 2), "active": active,
            }.items():
                rollout[key].append(np.array(data, copy=True))
            completed += int(following["done"].sum())
            result = following
            obs = observe()
        with torch.no_grad():
            bootstrap = model(tensor(obs, device))[2].cpu().numpy()
        data = {key: np.stack(value) for key, value in rollout.items()}
        adv, returns = advantages(data["reward"], data["value"], data["done"], bootstrap)
        selected = data["active"].reshape(-1)
        x = data["obs"].reshape(-1, data["obs"].shape[-1])[selected]
        masks0 = data["mask0"].reshape(-1, N_ACTIONS)[selected]
        masks1 = data["mask1"].reshape(-1, N_ACTIONS)[selected]
        actions0 = onehot(data["a0"].reshape(-1)[selected])
        actions1 = onehot(data["a1"].reshape(-1)[selected])
        old_logp = data["logp"].reshape(-1)[selected]
        target = returns.reshape(-1)[selected]
        advantage = adv.reshape(-1)[selected]
        advantage = (advantage - advantage.mean()) / max(float(advantage.std()), 1e-8)
        losses, entropies = [], []
        for _ in range(args.epochs):
            order = rng.permutation(len(x))
            for begin in range(0, len(x), args.batch_size):
                index = order[begin:begin + args.batch_size]
                hidden, logits0, prediction = model(tensor(x[index], device))
                lp0 = masked_logprobs(logits0, tensor(masks0[index], device))
                lp1 = masked_logprobs(model.second(hidden, tensor(actions0[index], device)), tensor(masks1[index], device))
                new_logp = (lp0 * tensor(actions0[index], device)).sum(-1) + (lp1 * tensor(actions1[index], device)).sum(-1)
                ratio = (new_logp - tensor(old_logp[index], device)).exp()
                weights = tensor(advantage[index], device)
                policy_loss = -torch.minimum(ratio * weights, ratio.clamp(0.8, 1.2) * weights).mean()
                value_loss = (prediction - tensor(target[index], device)).square().mean()
                entropy = -(lp0.exp() * lp0 + lp1.exp() * lp1).sum(-1).mean()
                loss = policy_loss + 0.5 * value_loss - 0.01 * entropy
                if not np.isfinite(float(loss.detach().cpu())):
                    raise RuntimeError("Non-finite PPO loss")
                optimizer.zero_grad(set_to_none=True)
                loss.backward()
                clip_gradients(model)
                optimizer.step()
                losses.append(float(loss.detach().cpu()))
                entropies.append(float(entropy.detach().cpu()))
        battles += completed
        metrics = {
            "update": update, "battles": battles, "completed_this_update": completed,
            "decisions": int(selected.sum()), "loss": float(np.mean(losses)), "entropy": float(np.mean(entropies)),
            "elapsed_seconds": time.perf_counter() - start, "device": str(device),
            "parameters_changed": any(not torch.equal(old, new.detach().cpu()) for old, new in zip(initial_weights, model.parameters())),
        }
        print(json.dumps(metrics), flush=True)
        with (args.output / "metrics.jsonl").open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(metrics) + "\n")
        save_checkpoint(args.output / "latest.pt", model, optimizer, args, update, battles, rng)
    if not metrics["parameters_changed"]:
        raise RuntimeError("Training completed without updating any model weights")
    print(f"Checkpoint: {args.output / 'latest.pt'}", flush=True)


if __name__ == "__main__":
    main()
