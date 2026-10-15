"""Gradient noise scale of a checkpoint's PPO gradients (McCandlish et al. 2018, "An Empirical Model of Large-Batch
Training", appendix A; formulas in training.noise).

    ../../.venv/bin/python noise_scale.py --checkpoint CKPT --out OUT.json [--rollouts 80]

Plays ``--rollouts`` rollouts with the checkpoint frozen (the trainer's opponent mix, snapshot envs as self-play, as
in learner_bench), computes the trainer's GAE advantages (normalised per rollout) and returns, then takes the
gradients of minibatches of ``--small`` rows drawn without replacement from each rollout's trained rows. At the
checkpoint the PPO ratio is 1, so the clipped surrogate's gradient is the plain policy gradient.

With g_i the small-batch gradients (size b) and G their mean (size B = m b):
  |G_true|^2 ~ (B |G|^2 - b mean|g_i|^2) / (B - b),   tr(Sigma) ~ (mean|g_i|^2 - |G|^2) / (1/b - 1/B),
  B_noise = tr(Sigma) / |G_true|^2: the batch size (decisions) at which gradient noise equals the signal.
``--grouping random`` (the default) draws the small batches as random rows, so tr(Sigma) is the per-row noise of
independent decisions; ``--grouping env`` cuts them as whole envs (training.noise.env_chunks, as the trainer's
``--noise-scale`` probe does), so tr(Sigma) also carries the correlation of a battle's decisions (the effective noise
of a rollout's rows, larger by the design effect). A jackknife over rollouts gives standard errors. Reported for the policy loss alone (the gradient that moves the
policy), the value loss alone, and the trainer's full loss (default coefficients, entropy coefficient
``--entropy-coef``). The first rollout starts every battle at turn 1 and is skipped. 80 rollouts of a 192x4 model take
~6 minutes on an M-series Mac.

Measured on 2026-10-15 (docs/training/SELFPLAY.md, "Noise scale and automatic learning rate"): policy B_noise 34k at
10 minutes of a 128x3 run, 37k at 60 minutes, 308k at m535 (192x4) and 211k for the 256x6 plateau average.
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
from training.league import SELF, SNAPSHOT, opponent_kinds
from training.model import Model
from training.noise import env_chunks, noise_estimates
from training.ppo import gae, tera_choice_entropy

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
KEYS = ("policy", "value", "full")


def rows_from(data: dict, gamma: float, lam: float) -> dict:
    """One rollout's trained rows as flat columns, with the trainer's normalised advantages and returns."""
    steps, R = data["a0"].shape
    adv, ret = gae(data["reward"], data["value"], data["done"], data["bootstrap"], gamma, lam)
    sel = np.flatnonzero(data["train"].reshape(-1))
    partner = (np.arange(steps)[:, None] * R + (np.arange(R) ^ 1)[None, :]).reshape(-1)[sel]
    a = adv.reshape(-1)[sel]
    cols = {k: data[k].reshape(-1, *data[k].shape[2:])[sel] for k in ts.OBS_KEYS}
    cols.update(flat=sel, adv=((a - a.mean()) / (a.std() + 1e-8)).astype(np.float32),
                returns=ret.reshape(-1)[sel].astype(np.float32),
                partner_ids=data["ids"].reshape(-1, *data["ids"].shape[2:])[partner],
                partner_floats=data["floats"].reshape(-1, *data["floats"].shape[2:])[partner][:, 0:6])
    for k in ("a0", "a1", "logp"):
        cols[k] = data[k].reshape(-1)[sel]
    for k in ("mask0", "mask1"):
        cols[k] = data[k].reshape(-1, ts.N_ACTIONS)[sel]
    return cols


def losses(model, b: dict, device, coefs) -> tuple[dict, torch.Tensor]:
    """The policy, value and full losses of one minibatch (the trainer's ppo_loss terms at ratio ~1)."""
    up = lambda a: torch.from_numpy(np.ascontiguousarray(a)).to(device)  # noqa: E731
    x = {k: up(b[k]) for k in ts.OBS_KEYS}
    state = model.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
    a0, a1 = up(b["a0"]), up(b["a1"])
    lp0, vectors0 = model.slot0(state, up(b["mask0"]))
    lp1 = model.slot1(state, vectors0, a0, up(b["mask1"]))
    new_logp = (lp0 * ts.onehot(a0)).sum(-1) + (lp1 * ts.onehot(a1)).sum(-1)
    ratio = (new_logp - up(b["logp"])).exp()
    policy = -(ratio * up(b["adv"])).mean()  # the clipped surrogate at ratio 1
    value = (model.value(state, up(b["partner_ids"]), up(b["partner_floats"])) - up(b["returns"])).square().mean()
    base0, _ = tera_choice_entropy(lp0)
    base1, _ = tera_choice_entropy(lp1)
    entropy = (base0 + base1).mean()
    aux = model.aux_loss(state, x["floats"], x["foe_match"], up(b["partner_ids"]))
    value_coef, entropy_coef, aux_coef = coefs
    full = policy + value_coef * value - entropy_coef * entropy + aux_coef * aux
    return dict(policy=policy, value=value, full=full), ratio


def flat_grad(model, loss) -> torch.Tensor:
    params = [p for p in model.parameters() if p.requires_grad]
    grads = torch.autograd.grad(loss, params, retain_graph=True, allow_unused=True)
    return torch.cat([(g if g is not None else torch.zeros_like(p)).reshape(-1) for g, p in zip(grads, params)])


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--checkpoint", required=True, type=Path)
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--rollouts", type=int, default=16)
    p.add_argument("--envs", type=int, default=128)
    p.add_argument("--steps", type=int, default=32)
    p.add_argument("--workers", type=int, default=4)
    p.add_argument("--small", type=int, default=256, help="rows per small batch")
    p.add_argument("--grouping", choices=("random", "env"), default="random",
                   help="small batches of random rows, or of whole envs (the trainer's probe)")
    p.add_argument("--pool", type=Path, default=REPO / "data/teams/train-s42-2000.txt")
    p.add_argument("--pool-fraction", type=float, default=0.3)
    p.add_argument("--scripted-share", type=float, default=0.33)
    p.add_argument("--entropy-coef", type=float, default=0.003, help="entropy coefficient in the full loss")
    p.add_argument("--seed", type=int, default=5)
    p.add_argument("--device", choices=BACKENDS, default="auto")
    args = p.parse_args()
    if args.rollouts < 2:
        p.error("--rollouts must be at least 2 (the jackknife leaves one out)")
    t0 = time.time()
    torch.manual_seed(args.seed)
    trainer = ts.parse_args([])
    coefs = (trainer.value_coef, args.entropy_coef, trainer.aux_coef)
    dex = load_dex()
    state = torch.load(args.checkpoint, map_location="cpu", weights_only=False)
    width, layers = state.get("width", 128), state.get("layers", 3)
    model = Model.from_state(dex, state["model"], width, layers).eval()
    device, _ = training_device(args.device)
    gpu_model = Model.from_state(dex, state["model"], width, layers).to(device).eval()
    rng = np.random.default_rng(args.seed)
    # Per rollout (a jackknife group): the sum of its small-batch gradients, of their squared norms, and the count.
    group_sum = {k: [] for k in KEYS}
    group_sq = {k: [] for k in KEYS}
    counts, rows_total, max_ratio = [], 0, 0.0
    kinds = opponent_kinds(args.envs, args.pool_fraction, args.scripted_share)
    kinds = np.where(kinds == SNAPSHOT, SELF, kinds).astype(np.int8)
    with Actors(args.envs, args.steps, [args.pool], args.seed, args.workers, model,
                scripted="smart", scripted_temperature=0.05) as actors:
        actors.set_weights("learner", ts.cpu_state(model))
        actors.set_weights("opponent", ts.cpu_state(model))
        for r in range(args.rollouts + 1):
            actors.start(r % 2, kinds, 0.0)
            actors.wait()
            if not r:
                continue
            raw = {k: np.array(v) for k, v in actors.buffers[r % 2].arrays.items()}
            data = rows_from(raw, trainer.gamma, trainer.lam)
            n = len(data["a0"])
            rows_total += n
            m = n // args.small
            order = (rng.permutation(n)[: m * args.small].reshape(m, args.small) if args.grouping == "random"
                     else env_chunks(data["flat"], raw["a0"].shape[1], args.small))
            sums = {k: None for k in KEYS}
            sq = {k: 0.0 for k in KEYS}
            for idx in order:
                terms, ratio = losses(gpu_model, {k: v[idx] for k, v in data.items()}, device, coefs)
                max_ratio = max(max_ratio, float((ratio.detach() - 1).abs().max()))
                for k in KEYS:
                    g = flat_grad(gpu_model, terms[k]).detach()
                    sq[k] += float(g.square().sum())
                    sums[k] = g if sums[k] is None else sums[k] + g
            for k in KEYS:
                group_sum[k].append(sums[k].cpu())
                group_sq[k].append(sq[k])
            counts.append(m)
            print(f"rollout {r}: {n} rows, {m} minibatches, {time.time() - t0:.0f} s", flush=True)
    counts = np.asarray(counts)
    result = dict(checkpoint=str(args.checkpoint), rows=rows_total, rollouts=args.rollouts, envs=args.envs,
                  small=args.small, grouping=args.grouping, minibatches=int(counts.sum()), max_ratio_dev=max_ratio,
                  params=int(sum(p.numel() for p in model.parameters())))

    def estimate(k, keep):
        m = int(counts[keep].sum())
        mean = torch.stack([group_sum[k][i] for i in keep]).sum(0) / m
        mean_square = sum(group_sq[k][i] for i in keep) / m
        return noise_estimates(mean_square, float(mean.square().sum()), args.small, m * args.small)

    G = len(counts)

    def jackknife_se(values) -> float:
        values = np.asarray(values)
        return float(np.sqrt((G - 1) / G * ((values - values.mean()) ** 2).sum()))

    for k in KEYS:
        g2, trace = estimate(k, list(range(G)))
        jack = [estimate(k, [j for j in range(G) if j != i]) for i in range(G)]
        ratios = np.asarray([t / g if g > 0 else np.inf for g, t in jack])
        result[k] = dict(g2=g2, trace=trace, b_noise=trace / g2 if g2 > 0 else None,
                         b_noise_jackknife_se=jackknife_se(ratios) if np.isfinite(ratios).all() else None,
                         g2_jackknife_se=jackknife_se([g for g, _ in jack]))
    result["seconds"] = round(time.time() - t0, 1)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=1))
    print(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
