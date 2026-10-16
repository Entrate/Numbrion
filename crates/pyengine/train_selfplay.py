"""Time-budgeted PPO self-play with the transformer policy (docs/training/TIPS.md, docs/training/SELFPLAY.md).

Run from the repository root:

    .venv\\Scripts\\python.exe crates\\pyengine\\train_selfplay.py --minutes 60

* Observations: training.encoder (own request + player-view protocol only), damage features (training.damage).
* Model: training.model (token transformer, per-action scoring, slot b conditioned on slot a, oracle critic,
  auxiliary hidden-information heads). ``--joint-features`` also gives slot b's move codes partner-aware damage/KO
  features from slot a's chosen code (training.model.JOINT_COLUMNS).
* Rollouts: actor processes (training.actors) play the battles with CPU copies of the policy and fill one of two
  shared-memory buffers while the GPU trains on the other, so each rollout is collected with the weights from one
  update earlier. PPO's ratio uses the stored behaviour log-probs.
* Move guards: the policy's masks block stale Fake Out / First Impression and repeated Protect-like moves unless
  no other move is legal (training.guards); scripted players keep the engine masks. ``--allow-repeat-protect``
  allows the repeat.
* Opponents (training.league): the learner plays both sides of most battles; ``--pool-fraction`` of the battles
  (fixed envs) put another player on side 2: ``--scripted-share`` of them a scripted player (``--scripted``: the
  doubles-aware smart player by default, or the strongest-attack heuristic), the rest an older snapshot (added every ``--pool-minutes``) chosen per rollout by prioritized
  fictitious self-play from the learner's results against each snapshot. Only the learner's own decisions are
  trained on. ``--search-fraction`` of the battles (taken from self-play) put shallow one-turn search with the
  learner's own weights on side 2 (training.search_opponent: the env's true battle, ``--search-config`` fields);
  its searched decisions become distillation label shards in ``--search-labels`` (default: ``--distill-stream``).
* Reward: +1 win, -1 loss, plus a potential-based KO-difference bonus (sums to zero over a battle) that decays
  linearly to zero at ``--ko-bonus-end`` of the run.
* Exploration: the entropy-bonus coefficient adapts so the summed slot-a + slot-b base-choice entropy tracks a target
  that falls linearly from ``--entropy-target`` to ``--entropy-target-final`` (``--entropy-target 0``: the fixed
  ``--entropy`` -> ``--entropy-final`` schedule instead). Tera gets no entropy bonus by default;
  ``--tera-entropy-weight 1`` restores the joint entropy bonus. ``tera_entropy`` logs its conditional contribution.
* Teacher (``--teacher-coef``, off by default): the smart player labels the learner's own trained decisions in the
  actors (training.actors; slot b's label answers the learner's sampled slot a), and the loss adds the weighted
  cross-entropy of those labels, DAgger-style imitation that falls linearly to zero at ``--teacher-end-minutes``;
  then the actors stop labelling. PPO's data (actions, behaviour log-probs, masks) is unchanged by it.
* Search distillation (``--distill DIR``, ``--distill-coef``, off by default): positions labelled offline by the
  search (search_labels.py), whose strategy over its candidate pairs the policy imitates (training.distill). Every
  minibatch adds ``--distill-batch`` labelled positions with weight ``--distill-coef``, falling linearly to zero at
  ``--distill-end-minutes``; held-out shards report ``distill_val_loss`` and ``distill_val_agree``.
* Continuous distillation (``--distill-stream DIR``): reuse training rows with update gaps, retire stale rows,
  validate on the newest holdout rows, and archive complete shards after latest.pt saves. Scan once per update.
* Schedule: learning rate, entropy and KO bonus anneal over ``--schedule-minutes`` (default: ``--minutes``).
  ``--minutes inf`` runs until <output>/STOP or ``--stop-minutes``. Snapshots every ``--snapshot-minutes`` feed
  evaluate.py. ``latest.pt`` (with optimizer, opponent-pool, entropy-controller and stream state) supports ``--resume``.
* Noise scale (``--noise-scale``, off by default): the gradient noise scale of the PPO policy loss, measured by a
  probe pass over env-grouped chunks of the rollout and smoothed over updates (training.noise), logged as ``noise_b``. ``--auto-lr``
  lowers the learning rate below the schedule (or sums more minibatches per step, ``--auto-lr-mode accumulate``) as
  it grows; ``latest.pt`` keeps both states.
* Metrics: besides the losses, win rates against snapshots and against the heuristic, the PFSP table, and the
  learner's action usage (Tera, switches, Protect, status moves, Fake Out; training.diagnostics).
"""
from __future__ import annotations

import argparse
import json
import math
import os
import time
from functools import lru_cache
from pathlib import Path

import numpy as np
import torch

from training.actors import Actors, memory_mb
from training.device import BACKENDS, training_device
from training.diagnostics import ActionUsage
from training.distill import DistillData, DistillStream, distill_loss, validate
from training.dex import load_dex
from training.guards import MoveGuards
from training.league import HEURISTIC, SEARCH, SELF, SNAPSHOT, SnapshotPool, opponent_kinds
from training.model import Model
from training.noise import GradientNoise, LrRule, NoiseScale, env_chunks
from training.ppo import Adam, EntropyController, WeightAverage, clip_gradients, gae, minibatches, tera_choice_entropy

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
OBS_KEYS = ("ids", "floats", "field", "foe_match", "action_features", "token_features")
N_ACTIONS = 47
CRITIC_PREFIXES = ("value_head.", "partner_proj.", "aux_")  # the weights --critic-warmup-updates trains


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


def opponent_results(collected: dict, info: dict) -> dict:
    """Finished games, the learner's score (not for self-play) and mean battle turns by opponent kind for one
    rollout, over all games that finished in the envs of that kind."""
    out = {}
    for kind, name in ((SELF, "self"), (SNAPSHOT, "pool"), (HEURISTIC, "scripted"), (SEARCH, "search")):
        envs = info["opponents"] == kind
        if kind == SEARCH and not envs.any():
            continue
        games = float(collected["games"][envs].sum())
        out[f"{name}_games"] = int(games)
        if kind != SELF:
            out[f"{name}_win_rate"] = round(float(collected["score"][envs].sum()) / games, 3) if games else None
        out[f"{name}_turns"] = round(float(collected["battle_turns"][envs].sum()) / games, 1) if games else None
    return out


def ppo_loss(model, batch: dict, partner_ids, partner_floats, a0, a1, mask0, mask1, old_logp, weight, returns,
             value_coef, entropy_weight, aux_coef, clip: float, tera_entropy_weight=0.0, teacher_a0=None,
             teacher_a1=None, teacher_labelled=None, teacher_coef=None):
    """One minibatch's PPO loss and its logged terms (policy, value, base/Tera entropy and auxiliary losses, KL, clip
    fraction), all on the device. ``--compile`` compiles this function.

    With ``teacher_a0`` (and ``teacher_a1``, ``teacher_labelled``, ``teacher_coef``) the loss adds ``teacher_coef``
    times the mean over labelled rows of ``-lp0[teacher_a0] - lp1[teacher_a1]``, and the terms end with that mean.
    ``lp1`` is conditioned on the stored slot-a code ``a0``, the one the slot-b label answers (training.actors), so
    no extra forward pass is needed. The caller passes unlabelled rows as code 0 with ``teacher_labelled`` 0 (a float
    mask, so no int64 compare or clamp runs on the device; DirectML falls back to the CPU for some). A masked code's
    log-prob is the finite -1e9 fill, so zero-weighted rows add exactly zero; a minibatch without labels gives 0."""
    state = model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"], batch["token_features"])
    lp0, vectors0 = model.slot0(state, mask0)
    lp1 = model.slot1(state, vectors0, a0, mask1)
    new_logp = (lp0 * onehot(a0)).sum(-1) + (lp1 * onehot(a1)).sum(-1)
    ratio = (new_logp - old_logp).exp()
    policy_loss = -torch.minimum(ratio * weight, ratio.clamp(1 - clip, 1 + clip) * weight).mean()
    value = model.value(state, partner_ids, partner_floats)
    value_loss = (value - returns).square().mean()
    base0, tera0 = tera_choice_entropy(lp0)
    base1, tera1 = tera_choice_entropy(lp1)
    entropy = (base0 + base1).mean()
    tera_entropy = (tera0 + tera1).mean()
    aux_loss = model.aux_loss(state, batch["floats"], batch["foe_match"], partner_ids)
    bonus = entropy + tera_entropy_weight * tera_entropy
    loss = policy_loss + value_coef * value_loss - entropy_weight * bonus + aux_coef * aux_loss
    r, log_ratio = ratio.detach(), (new_logp - old_logp).detach()
    kl, clipfrac = ((r - 1) - log_ratio).mean(), ((r - 1).abs() > clip).float().mean()
    terms = (policy_loss.detach(), value_loss.detach(), entropy.detach(), tera_entropy.detach(), aux_loss.detach(), kl,
             clipfrac)
    if teacher_a0 is not None:
        nll = -(lp0 * onehot(teacher_a0)).sum(-1) - (lp1 * onehot(teacher_a1)).sum(-1)
        teacher_loss = (nll * teacher_labelled).sum() / teacher_labelled.sum().clamp(min=1)
        loss = loss + teacher_coef * teacher_loss
        terms = terms + (teacher_loss.detach(),)
    return loss, terms


def policy_surrogate(model, batch: dict, a0, a1, mask0, mask1, old_logp, weight):
    """The unclipped PPO surrogate ``-mean(ratio * advantage)``, whose gradient is the importance-weighted policy
    gradient: ppo_loss's policy term without the clip, and without the critic and auxiliary heads (the noise-scale
    probe differentiates it). ``old_logp`` None takes ``model``'s own probabilities as the baseline (ratio 1, the
    plain policy gradient ``-mean(advantage * grad log p)`` at ``model``'s weights)."""
    state = model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"], batch["token_features"])
    lp0, vectors0 = model.slot0(state, mask0)
    lp1 = model.slot1(state, vectors0, a0, mask1)
    new_logp = (lp0 * onehot(a0)).sum(-1) + (lp1 * onehot(a1)).sum(-1)
    if old_logp is None:
        old_logp = new_logp.detach()
    return -((new_logp - old_logp).exp() * weight).mean()


@lru_cache(maxsize=1)
def compiled_ppo_loss():
    import torch._inductor.config

    # Metal allows 31 buffer arguments per kernel; unbounded fusion generated kernels with more (torch 2.14.1).
    torch._inductor.config.max_fusion_unique_io_buffers = 24
    return torch.compile(ppo_loss)


def teacher_coefficient(args, elapsed: float) -> float:
    """Imitation weight after ``elapsed`` training seconds (a resumed run counts its earlier time, like the other
    schedules): ``--teacher-coef`` falling linearly to zero at ``--teacher-end-minutes``."""
    end = getattr(args, "teacher_end_minutes", 0.0) * 60
    if getattr(args, "teacher_coef", 0.0) <= 0 or end <= 0:
        return 0.0
    return args.teacher_coef * max(0.0, 1 - elapsed / end)


def distill_coefficient(args, elapsed: float) -> float:
    """Search-distillation weight after ``elapsed`` training seconds: ``--distill-coef`` falling linearly to zero at
    ``--distill-end-minutes``."""
    end = getattr(args, "distill_end_minutes", 0.0) * 60
    if getattr(args, "distill_coef", 0.0) <= 0 or end <= 0:
        return 0.0
    return args.distill_coef * max(0.0, 1 - elapsed / end)


def schedule_progress(args, elapsed: float) -> float:
    minutes = getattr(args, "schedule_minutes", None)
    minutes = args.minutes if minutes is None else minutes
    return min(max(elapsed / (minutes * 60), 0.0), 1.0)


def ppo_update(model, optimizer, data: dict, args, rng, entropy_coef: float, device,
               teacher_coef: float = 0.0, distill: DistillData | None = None,
               distill_coef: float = 0.0, elapsed: float = 0.0, update: int | None = None,
               noise: GradientNoise | None = None, accumulate: int = 1,
               probe_model: torch.nn.Module | None = None) -> tuple[dict, int]:
    """PPO with the oracle critic and the auxiliary loss on one rollout buffer of [T, rows] arrays.

    With ``--teacher-coef`` set, the buffer carries teacher labels and the loss imitates them with weight
    ``teacher_coef`` (see ppo_loss). The term stays in the loss for the whole run, at weight 0 and without labels after
    the fade, so a compiled loss keeps one graph.

    With ``distill`` and ``distill_coef`` > 0, every minibatch also imitates ``--distill-batch`` search-labelled
    positions (training.distill), uncompiled, with weight ``distill_coef``. A stream instead spreads at most
    ``--distill-max-batches`` batches of eligible rows over the update, and starvation leaves plain PPO unchanged.

    With ``noise`` (training.noise.GradientNoise), a probe pass before the first step adds to it the gradients of the
    unclipped PPO surrogate (the importance-weighted policy gradient) on chunks of ``--minibatch`` trained rows that
    keep each env's rows together (training.noise.env_chunks), taken at ``probe_model``'s weights (default:
    ``model``'s; the trainer passes the weight average, and there the probe drops the importance ratio to the
    rollout policy: a lagging average makes those ratios spread, which inflated the trace and buried the signal);
    it changes neither the weights nor ``rng``. Not during the
    critic warm-up. ``accumulate`` > 1 sums that many minibatch gradients (their
    mean) per optimizer step, the last step of the update taking what is left (``--auto-lr-mode accumulate``)."""
    steps, R = data["a0"].shape
    partner = np.arange(R) ^ 1
    advantages, returns = gae(data["reward"], data["value"], data["done"], data["bootstrap"], args.gamma, args.lam)
    selected = np.flatnonzero(data["train"].reshape(-1))
    flat = {k: data[k].reshape(-1, *data[k].shape[2:]) for k in OBS_KEYS}
    partner_index = (np.arange(steps)[:, None] * R + partner[None, :]).reshape(-1)
    adv = advantages.reshape(-1)[selected]
    adv = (adv - adv.mean()) / (adv.std() + 1e-8)
    returns_all = returns.reshape(-1)
    columns = dict(flat, a0=data["a0"].reshape(-1), a1=data["a1"].reshape(-1),
                   mask0=data["mask0"].reshape(-1, N_ACTIONS), mask1=data["mask1"].reshape(-1, N_ACTIONS),
                   logp=data["logp"].reshape(-1), returns=returns_all.astype(np.float32))
    weights = adv.astype(np.float32)
    teach = getattr(args, "teacher_coef", 0.0) > 0
    if teach:
        labels = data["teacher_a0"].reshape(-1), data["teacher_a1"].reshape(-1)
        labelled = labels[0] >= 0
        rated = selected[labelled[selected]]
        if not all(columns[f"mask{s}"][rated, labels[s][rated]].all() for s in (0, 1)):
            raise RuntimeError("A teacher label is illegal under the stored policy mask")
        columns.update(teacher_a0=np.maximum(labels[0], 0), teacher_a1=np.maximum(labels[1], 0),
                       teacher_labelled=labelled.astype(np.float32))
    terms_keys = ("policy_loss", "value_loss", "entropy", "tera_entropy", "aux_loss", "kl", "clipfrac",
                  *(("teacher_loss",) if teach else ()))
    # --critic-warmup-updates: only the critic and the hidden-information heads learn (a model copied by imitation
    # has untrained ones); every other weight keeps its value, and no distillation rows are drawn (they would be
    # spent without teaching the policy). The trainer passes the critic heads' own optimizer for these updates.
    warming = update is not None and update <= getattr(args, "critic_warmup_updates", 0)
    distilling = distill is not None and distill_coef > 0 and not warming
    stream_batches = None
    if isinstance(distill, DistillStream):
        n_minibatches = args.epochs * sum(1 for _ in minibatches(np.arange(len(selected)), args.minibatch))
        stream_batches = distill.begin_update(rng, args.distill_batch,
                                              getattr(args, "distill_max_batches", 4) if distilling else 0,
                                              n_minibatches, elapsed / 60, update=update)
    stats = {k: [] for k in (*terms_keys, "grad_norm", *(("distill_loss", "distill_agree") if distilling else ()))}
    # Loss coefficients as device tensors: the entropy coefficient changes every update, and DirectML keeps host
    # memory for every op given a new Python scalar (see training.ppo.Adam).
    dtype = next(model.parameters()).dtype
    value_coef, entropy_weight, aux_coef, tera_entropy_weight = (
        torch.full((), float(x), device=device, dtype=dtype)
        for x in (args.value_coef, entropy_coef, args.aux_coef, args.tera_entropy_weight))
    teacher_weight = torch.full((), float(teacher_coef), device=device, dtype=dtype) if teach else None
    distill_weight = torch.full((), float(distill_coef), device=device, dtype=dtype) if distilling else None
    loss_fn = compiled_ppo_loss() if getattr(args, "compile", False) else ppo_loss
    frozen = [p for name, p in model.named_parameters() if not name.startswith(CRITIC_PREFIXES)] if warming else []
    probing = noise is not None and not warming
    probe_model = model if probe_model is None else probe_model
    probed = [p for p in probe_model.parameters() if p.requires_grad] if probing else []

    def up(array):
        return torch.from_numpy(np.ascontiguousarray(array)).to(device)

    # MPS shares memory with the CPU: copy the whole rollout once per update and gather the minibatches on the
    # device, instead of gathering them with NumPy and copying each (0.14 s of a 2.3 s update).
    resident = device.type == "mps"
    if resident:
        columns, weights = {k: up(v) for k, v in columns.items()}, up(weights)

    def rows(key, index, extra=()):
        return columns[key][(index, *extra)] if resident else up(columns[key][(index, *extra)])

    means = {}  # 1 / summed as device tensors (DirectML keeps host memory for every op given a Python scalar)

    def step(summed: int) -> None:
        if summed > 1:  # the mean of the summed minibatch gradients
            if summed not in means:
                means[summed] = torch.full((), 1.0 / summed, device=device, dtype=dtype)
            for p in optimizer.parameters:
                if p.grad is not None:
                    p.grad.mul_(means[summed])
        norm = clip_gradients(optimizer.parameters, args.max_grad_norm)
        stats["grad_norm"].append(norm)
        optimizer.step(skip=~torch.isfinite(norm))  # a non-finite gradient leaves the weights as they were

    def minibatch_loss(pick):
        """ppo_loss on the trained rows at positions ``pick`` of ``selected``."""
        index, partner_rows = selected[pick], partner_index[selected[pick]]
        if resident:
            pick, index, partner_rows = up(pick), up(index), up(partner_rows)
        batch = {k: rows(k, index) for k in OBS_KEYS}
        teacher = (dict(teacher_a0=rows("teacher_a0", index), teacher_a1=rows("teacher_a1", index),
                        teacher_labelled=rows("teacher_labelled", index), teacher_coef=teacher_weight)
                   if teach else {})
        return loss_fn(model, batch, rows("ids", partner_rows), rows("floats", partner_rows, (slice(0, 6),)),
                       rows("a0", index), rows("a1", index), rows("mask0", index), rows("mask1", index),
                       rows("logp", index), weights[pick] if resident else up(weights[pick]), rows("returns", index),
                       value_coef, entropy_weight, aux_coef, args.clip, tera_entropy_weight, **teacher)

    if probing:  # the noise-scale probe: whole envs per chunk, before the first step (training.noise)
        for chunk in env_chunks(selected, R, args.minibatch):
            index = selected[chunk]
            if resident:
                chunk, index = up(chunk), up(index)
            surrogate = policy_surrogate(probe_model, {k: rows(k, index) for k in OBS_KEYS}, rows("a0", index),
                                         rows("a1", index), rows("mask0", index), rows("mask1", index),
                                         rows("logp", index) if probe_model is model else None,
                                         weights[chunk] if resident else up(weights[chunk]))
            noise.add(torch.autograd.grad(surrogate, probed, allow_unused=True), len(index))

    minibatch_index = summed = 0
    for _ in range(args.epochs):
        for pick in minibatches(rng.permutation(len(selected)), args.minibatch):
            loss, terms = minibatch_loss(pick)
            if distilling and (stream_batches is None or minibatch_index in stream_batches):
                labels_batch = (distill.sample(rng, args.distill_batch) if stream_batches is None
                                else stream_batches.pop(minibatch_index))
                tilt = getattr(args, "distill_tilt", 0.0)
                imitation, agree = distill_loss(model, labels_batch, tilt) if tilt > 0 else distill_loss(model, labels_batch)
                loss = loss + distill_weight * imitation
                stats["distill_loss"].append(imitation.detach())
                stats["distill_agree"].append(agree)
            minibatch_index += 1
            if not summed:
                optimizer.zero_grad()
            loss.backward()
            for p in frozen:
                p.grad = None  # Adam and the clipping skip parameters without a gradient
            summed += 1
            if summed == accumulate:
                step(summed)
                summed = 0
            # Statistics stay on the device and are read back once per update (a .cpu() per value per minibatch
            # stalls the DirectML queue, ~10% of the update).
            for key, item in zip(terms_keys, terms):
                stats[key].append(item)
    if summed:
        step(summed)
    stats = {k: torch.stack(v).cpu().numpy() if v else np.zeros(1) for k, v in stats.items()}
    if not np.isfinite(stats["policy_loss"]).all() or not np.isfinite(stats["grad_norm"]).all():
        raise RuntimeError("Non-finite PPO loss or gradient norm in this update (the optimizer skipped those steps, "
                           "so the weights are still finite)")
    values, rets = data["value"].reshape(-1)[selected], returns_all[selected]
    summary = {k: round(float(np.mean(v)), 4) for k, v in stats.items()}
    summary["explained_variance"] = round(float(1 - np.var(rets - values) / (np.var(rets) + 1e-8)), 3)
    if distill is not None:
        summary["distill_coef"] = round(float(distill_coef), 5)
        summary.setdefault("distill_loss", None)
        summary.setdefault("distill_agree", None)
        if stream_batches is not None:
            summary.update(distill.metrics)
            if not distill.metrics["distill_batches"]:
                summary["distill_loss"] = summary["distill_agree"] = None
    if teach:
        # Agreement of the learner's sampled codes with the labels, over slot decisions with a choice (forced ones
        # agree trivially); slot b's label answers the learner's own slot a.
        summary["teacher_coef"] = round(float(teacher_coef), 5)
        if not len(rated):
            summary["teacher_loss"] = None
        for s in (0, 1):  # from the host arrays (on MPS, ``columns`` now holds device tensors)
            choice = rated[data[f"mask{s}"].reshape(-1, N_ACTIONS)[rated].sum(-1) > 1]
            agree = (data[f"a{s}"].reshape(-1)[choice] == labels[s][choice]).mean() if len(choice) else None
            summary[f"teacher_agree{s}"] = None if agree is None else round(float(agree), 4)
    return summary, len(selected)


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--minutes", type=float, default=60.0, help="training wall-clock budget (inf: until STOP)")
    parser.add_argument("--schedule-minutes", type=float,
                        help="duration of progress-based schedules (default: --minutes; progress clamps at 1)")
    parser.add_argument("--stop-minutes", type=float,
                        help="stop after this many minutes while the schedules still span --schedule-minutes (to reproduce "
                             "the start of a longer run)")
    parser.add_argument("--device", choices=BACKENDS, default="auto")
    parser.add_argument("--adapter", type=int, default=0)
    parser.add_argument("--pool", type=Path, default=REPO / "data/teams/train-s42-2000.txt")
    parser.add_argument("--envs", type=int, default=128)
    parser.add_argument("--workers", type=int, default=4, help="actor processes (the learner is the bottleneck; leave cores for it)")
    parser.add_argument("--steps", type=int, default=32, help="decision boundaries per rollout")
    parser.add_argument("--allow-repeat-protect", action="store_true",
                        help="allow a Protect-like move right after a successful one, blocked by default since it "
                             "works one time in three")
    parser.add_argument("--epochs", type=int, default=2)
    parser.add_argument("--minibatch", type=int, default=1024)
    parser.add_argument("--width", type=int, default=128)
    parser.add_argument("--layers", type=int, default=3)
    parser.add_argument("--joint-features", action="store_true",
                        help="slot b's move codes see damage/KO features combined with slot a's chosen code")
    parser.add_argument("--lr", type=float, default=3e-4)
    parser.add_argument("--lr-final", type=float, default=0.3, help="final learning-rate multiplier")
    parser.add_argument("--gamma", type=float, default=0.995)
    parser.add_argument("--lam", type=float, default=0.95)
    parser.add_argument("--clip", type=float, default=0.2)
    parser.add_argument("--entropy", type=float, default=0.01,
                        help="entropy-bonus coefficient (the starting value of the adaptive coefficient)")
    parser.add_argument("--entropy-final", type=float, default=0.003,
                        help="final coefficient of the fixed linear schedule (only with --entropy-target 0)")
    parser.add_argument("--tera-entropy-weight", type=float, default=0.0,
                        help="conditional Tera entropy weight in the bonus (0: base only; 1: old joint bonus)")
    parser.add_argument("--entropy-target", type=float, default=2.5,
                        help="target for summed slot-a + slot-b base entropy at the start (0: fixed schedule)")
    parser.add_argument("--entropy-target-final", type=float, default=1.5, help="entropy target at the end of the run")
    parser.add_argument("--entropy-rate", type=float, default=0.2,
                        help="change of log(coefficient) per update and nat of entropy error")
    parser.add_argument("--entropy-coef-min", type=float, default=0.001)
    parser.add_argument("--entropy-coef-max", type=float, default=0.05)
    parser.add_argument("--critic-warmup-updates", type=int, default=0,
                        help="for the run's first N updates train only the critic and hidden-information heads (for "
                             "a model copied by imitation, whose heads are untrained)")
    parser.add_argument("--value-coef", type=float, default=0.5)
    parser.add_argument("--aux-coef", type=float, default=0.03)
    parser.add_argument("--max-grad-norm", type=float, default=0.5)
    parser.add_argument("--teacher-coef", type=float, default=0.0,
                        help="imitation loss weight on the smart player's labels for the learner's own decisions "
                             "(0: off, nothing is labelled); falls linearly to 0 at --teacher-end-minutes")
    parser.add_argument("--teacher-end-minutes", type=float, default=30.0,
                        help="elapsed training minutes at which the imitation weight reaches 0 and labelling stops")
    parser.add_argument("--teacher-temperature", type=float, default=0.0,
                        help="Gumbel noise on the teacher's pair values (0: its best pair)")
    labels = parser.add_mutually_exclusive_group()
    labels.add_argument("--distill", type=Path, nargs="+",
                        help="search_labels.py output directories (or label files) for search distillation")
    labels.add_argument("--distill-stream", type=Path, help="directory of incoming label shards with bounded reuse")
    parser.add_argument("--distill-coef", type=float, default=0.0,
                        help="search-distillation loss weight (0: off); falls linearly to 0 at --distill-end-minutes")
    parser.add_argument("--distill-end-minutes", type=float, default=60.0)
    parser.add_argument("--distill-batch", type=int, default=128, help="labelled positions per minibatch")
    parser.add_argument("--distill-tilt", type=float, default=0.0,
                        help="> 0: imitate the search's values applied to the current policy, pi * exp(q / tilt) over the candidates, instead of the labelling teacher's strategy (training.distill)")
    parser.add_argument("--distill-max-batches", type=int, default=4, help="maximum stream batches per PPO update")
    parser.add_argument("--distill-max-uses", type=int, default=4, help="maximum batches per stream row over its life")
    parser.add_argument("--distill-reuse-gap", type=int, default=50, help="minimum PPO updates between stream row uses")
    parser.add_argument("--distill-max-age-minutes", type=float, default=60.0,
                        help="retire stream rows older than this trainer-minute lag (inf: no retirement)")
    parser.add_argument("--distill-max-resident-rows", type=int, default=60000,
                        help="maximum unfinished stream training rows cached in memory; remaining rows wait on disk")
    parser.add_argument("--distill-holdout-rows", type=int, default=4000, help="newest stream rows kept for validation")
    parser.add_argument("--distill-pairs", type=int, default=4,
                        help="most weighted search candidates per position that the loss imitates")
    parser.add_argument("--distill-validate-every", type=int, default=25,
                        help="updates between held-out distillation losses in the metrics")
    parser.add_argument("--ko-bonus", type=float, default=0.02,
                        help="potential-based bonus per KO difference (sums to zero over a battle)")
    parser.add_argument("--ko-bonus-end", type=float, default=0.25,
                        help="fraction of the run at which the KO bonus has decayed linearly to zero")
    parser.add_argument("--pool-fraction", type=float, default=0.3,
                        help="share of battles against a snapshot or the heuristic instead of pure self-play")
    parser.add_argument("--scripted-share", type=float, default=0.33,
                        help="share of those battles against the scripted player")
    parser.add_argument("--scripted", choices=["smart", "heuristic"], default="smart",
                        help="scripted opponent: the doubles-aware smart player (training.smart) or the "
                             "strongest-attack heuristic")
    parser.add_argument("--scripted-temperature", type=float, default=0.05,
                        help="Gumbel noise on the smart player's pair values, so it cannot be memorized")
    parser.add_argument("--search-fraction", type=float, default=0.0,
                        help="share of battles (taken from self-play) against shallow search with the learner's weights")
    parser.add_argument("--search-config", default="{}",
                        help="training.search_opponent.OpponentSearch fields as JSON (default: 4x3 pairs, 1 seed)")
    parser.add_argument("--search-labels", type=Path,
                        help="directory for the search opponent's label shards (default: --distill-stream; none without)")
    parser.add_argument("--pool-minutes", type=float, default=3.0)
    parser.add_argument("--pool-size", type=int, default=12)
    parser.add_argument("--pfsp-power", type=float, default=2.0,
                        help="PFSP: snapshot weight (1 - learner score EMA) ** power")
    parser.add_argument("--pfsp-floor", type=float, default=0.1, help="PFSP: uniform share mixed into the weights")
    parser.add_argument("--pfsp-alpha", type=float, default=0.02, help="PFSP: per-game EMA rate of the learner score")
    parser.add_argument("--snapshot-minutes", type=float, default=5.0)
    parser.add_argument("--ema-updates", type=float, default=0.0,
                        help="time constant (in updates) of an exponential moving average of the weights, saved as "
                             "OUTPUT/ema/mNNN.pt beside every snapshot and as OUTPUT/ema/latest.pt (0: off). Late in a "
                             "run the average plays better than the raw weights; evaluate those files")
    noise = parser.add_argument_group("noise scale and automatic learning rate (training.noise; off by default)")
    noise.add_argument("--noise-scale", action="store_true",
                       help="measure the gradient noise scale of the PPO policy loss (a probe pass, forward and "
                            "policy backward, over the rollout) and log it as noise_b (implied by --auto-lr)")
    noise.add_argument("--noise-every", type=int, default=4, help="measure every that many updates")
    noise.add_argument("--noise-weights", choices=("average", "raw"),
                       help="measure at the weight average (default with --ema-updates) or at the raw training weights "
                            "(default without; their noise scale falls as the learning rate rises: see the docs)")
    noise.add_argument("--noise-ema", type=float, default=50.0,
                       help="time constant (in measured updates) of the noise-scale EMAs")
    noise.add_argument("--auto-lr", action="store_true",
                       help="lower the step temperature as the noise scale grows: factor = clamp((B_ref / B_noise) ** "
                            "power, --auto-lr-min, 1) times the scheduled learning rate (or a bigger batch, see "
                            "--auto-lr-mode)")
    noise.add_argument("--auto-lr-mode", choices=LrRule.MODES, default="lr",
                       help="lr: multiply the learning rate by the factor; accumulate: keep it and sum round(1 / "
                            "factor) minibatch gradients per optimizer step")
    noise.add_argument("--auto-lr-power", type=float, default=0.5,
                       help="exponent of B_ref / B_noise (0.5: Adam's square-root rule; 1: linear, SGD)")
    noise.add_argument("--auto-lr-min", type=float, default=0.1, help="smallest factor")
    noise.add_argument("--auto-lr-max-change", type=float, default=0.02,
                       help="largest relative change of the factor per measured update")
    noise.add_argument("--auto-lr-bref", type=float,
                       help="reference noise scale B_ref in rows (default: the smoothed one after --auto-lr-warmup "
                            "measured updates; a given value also overrides a resumed one)")
    noise.add_argument("--auto-lr-warmup", type=int, default=50,
                       help="measured updates before B_ref is taken (without --auto-lr-bref)")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--output", type=Path, default=REPO / "scratch/training/selfplay")
    parser.add_argument("--resume", type=Path)
    parser.add_argument("--compile", action="store_true",
                        help="torch.compile the minibatch loss (~12 s of compiling per minibatch size at the start)")
    args = parser.parse_args(argv)
    if args.schedule_minutes is None:
        args.schedule_minutes = args.minutes
    for name in ("minutes", "schedule_minutes", "snapshot_minutes", "pool_minutes"):
        value = getattr(args, name)
        if math.isnan(value) or value <= 0:
            parser.error(f"--{name.replace('_', '-')} must be positive (inf is allowed)")
    if args.stop_minutes is not None and (math.isnan(args.stop_minutes) or args.stop_minutes < 0):
        parser.error("--stop-minutes must be nonnegative (inf is allowed)")
    if args.teacher_coef < 0 or args.teacher_temperature < 0:
        parser.error("--teacher-coef and --teacher-temperature must be nonnegative")
    if args.distill_coef < 0 or (args.distill_coef > 0 and not (args.distill or args.distill_stream)):
        parser.error("--distill-coef must be nonnegative and needs --distill or --distill-stream")
    if math.isnan(args.distill_tilt) or args.distill_tilt < 0:
        parser.error("--distill-tilt must be nonnegative")
    if args.distill_batch <= 0 or args.distill_pairs <= 0 or args.distill_validate_every <= 0:
        parser.error("--distill-batch, --distill-pairs and --distill-validate-every must be positive")
    if args.distill_max_batches < 0 or args.distill_holdout_rows < 0:
        parser.error("--distill-max-batches and --distill-holdout-rows must be nonnegative")
    if args.distill_max_uses <= 0 or args.distill_reuse_gap < 0:
        parser.error("--distill-max-uses must be positive and --distill-reuse-gap must be nonnegative")
    if math.isnan(args.distill_max_age_minutes) or args.distill_max_age_minutes < 0:
        parser.error("--distill-max-age-minutes must be nonnegative (inf is allowed)")
    if not 0 <= args.search_fraction <= 1 or args.search_fraction + args.pool_fraction > 1:
        parser.error("--search-fraction must be in [0, 1] and fit beside --pool-fraction")
    try:
        from training.search_opponent import OpponentSearch

        args.search_config = json.loads(args.search_config)
        OpponentSearch(**args.search_config)
    except (ValueError, TypeError) as error:
        parser.error(f"--search-config: {error}")
    if args.search_labels is None and args.search_fraction > 0:
        args.search_labels = args.distill_stream
    if args.ema_updates != 0 and not args.ema_updates >= 1:
        parser.error("--ema-updates must be 0 (off) or at least 1")
    if args.noise_weights == "average" and not args.ema_updates:
        parser.error("--noise-weights average needs --ema-updates")
    if args.auto_lr and args.noise_weights is None and not args.ema_updates:
        parser.error("--auto-lr measures the noise scale at the weight average: give --ema-updates (or --noise-weights "
                     "raw, whose noise scale grows as the rule lowers the learning rate)")
    if args.noise_weights is None:
        args.noise_weights = "average" if args.ema_updates else "raw"
    if args.auto_lr:
        args.noise_scale = True
        try:
            LrRule(args.auto_lr_power, args.auto_lr_min, args.auto_lr_max_change, args.auto_lr_bref,
                   args.auto_lr_warmup, args.auto_lr_mode)
        except ValueError as error:
            parser.error(f"--auto-lr: {error}")
    if args.noise_every < 1 or not args.noise_ema >= 1:
        parser.error("--noise-every and --noise-ema must be at least 1")
    if args.distill_max_resident_rows <= 0 or (args.distill_stream and args.distill_max_resident_rows < args.distill_batch):
        parser.error("--distill-max-resident-rows must be positive and fit --distill-batch for a stream")
    return args


def main():
    args = parse_args()

    torch.manual_seed(args.seed)
    torch.set_num_threads(1)
    rng = np.random.default_rng(args.seed)
    device, adapter = training_device(args.device, args.adapter)
    dex = load_dex()
    model = Model(dex, args.width, args.layers, joint_features=args.joint_features).to(device)
    optimizer = Adam(model.parameters(), lr=args.lr)
    pool = SnapshotPool(args.pool_size, args.pfsp_power, args.pfsp_floor, args.pfsp_alpha)
    controller = (EntropyController(args.entropy, args.entropy_target, args.entropy_target_final, args.entropy_rate,
                                    args.entropy_coef_min, args.entropy_coef_max) if args.entropy_target > 0 else None)
    usage = ActionUsage(dex)
    noise_scale = NoiseScale(args.noise_ema, args.noise_every) if args.noise_scale else None
    lr_rule = (LrRule(args.auto_lr_power, args.auto_lr_min, args.auto_lr_max_change, args.auto_lr_bref,
                      args.auto_lr_warmup, args.auto_lr_mode) if args.auto_lr else None)
    elapsed_before = 0.0
    update = battles = samples_total = 0
    state = {}
    if args.resume:
        state = torch.load(args.resume, map_location="cpu", weights_only=False)
        saved = state["model"]["act_damage.weight"].shape[1], state["model"]["float_proj.weight"].shape[1]
        if saved != (model.act_damage.in_features, model.float_proj.in_features):
            raise SystemExit(f"{args.resume} was trained on other damage-calc feature columns (training.damage); "
                             "start a new run (evaluate.py still plays it against new models)")
        if ("act_joint.weight" in state["model"]) != args.joint_features:
            raise SystemExit(f"{args.resume} was trained {'without' if args.joint_features else 'with'} "
                             "--joint-features; resume it with the flag it was trained with")
        model.load_state_dict(state["model"])
        optimizer.load_state_dict(state["optimizer"])
        pool.load_state_dict(state["pool"])
        if controller is not None and state.get("entropy_controller"):
            controller.load_state_dict(state["entropy_controller"])
        if noise_scale is not None and state.get("noise_scale"):
            noise_scale.load_state_dict(state["noise_scale"])
        if lr_rule is not None and state.get("lr_rule"):
            lr_rule.load_state_dict(state["lr_rule"], keep_b_ref=args.auto_lr_bref is not None)
        elapsed_before, update, battles, samples_total = state["elapsed"], state["update"], state["battles"], state["samples"]
        rng.bit_generator.state = state["numpy_rng"]
    average = WeightAverage(model, args.ema_updates) if args.ema_updates > 0 else None
    if average is not None and state.get("ema") is not None:
        average.load_state_dict(state["ema"])  # else it starts from the resumed weights
    probe_model = None  # --noise-weights average: the noise-scale probe runs on a copy holding the weight average
    if noise_scale is not None and args.noise_weights == "average":
        probe_model = Model(dex, args.width, args.layers, joint_features=args.joint_features).to(device)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "config.json").write_text(json.dumps({k: str(v) for k, v in vars(args).items()}, indent=1))
    n_params = sum(p.numel() for p in model.parameters())
    print(json.dumps({"adapter": adapter, "device": str(device), "parameters": n_params}), flush=True)
    distill = None
    if args.distill_stream:
        distill = DistillStream(args.distill_stream, device, pairs=args.distill_pairs,
                                holdout_rows=args.distill_holdout_rows, state=state.get("distill_stream"),
                                max_resident_rows=args.distill_max_resident_rows, max_uses=args.distill_max_uses,
                                reuse_gap=args.distill_reuse_gap, max_age_minutes=args.distill_max_age_minutes,
                                update=update, elapsed_minutes=elapsed_before / 60)
        print(json.dumps({"distill": distill.describe()}), flush=True)
    elif args.distill and args.distill_coef > 0:
        distill = DistillData(args.distill, device, pairs=args.distill_pairs)
        print(json.dumps({"distill": distill.describe()}), flush=True)

    kinds = opponent_kinds(args.envs, args.pool_fraction, args.scripted_share,
                           args.search_fraction)  # fixed per env for the whole run
    budget = args.minutes * 60
    stop = min(budget, args.stop_minutes * 60) if args.stop_minutes is not None else budget
    next_pool = elapsed_before + args.pool_minutes * 60 if len(pool) else 0.0
    snapshot_seconds = args.snapshot_minutes * 60
    next_snapshot = ((int(elapsed_before // snapshot_seconds) + (1 if elapsed_before else 0)) * snapshot_seconds
                     if math.isfinite(snapshot_seconds) else math.inf)
    loaded = {"snapshot": None}  # id of the snapshot in the actors' opponent weights
    critic_optimizer = None  # --critic-warmup-updates (a run resumed during the warm-up starts it afresh)

    def checkpoint(elapsed):
        save(args.output / "latest.pt", {
            "model": cpu_state(model), "optimizer": optimizer.state_dict(), "pool": pool.state_dict(),
            "entropy_controller": controller.state_dict() if controller is not None else None, "elapsed": elapsed,
            "update": update, "battles": battles, "samples": samples_total, "numpy_rng": rng.bit_generator.state,
            "width": args.width, "layers": args.layers,
            **({"distill_stream": distill.state_dict()} if isinstance(distill, DistillStream) else {}),
            **({"ema": average.state_dict()} if average is not None else {}),
            **({"noise_scale": noise_scale.state_dict()} if noise_scale is not None else {}),
            **({"lr_rule": lr_rule.state_dict()} if lr_rule is not None else {})})
        if average is not None:
            save(args.output / "ema" / "latest.pt", {"model": average.state_dict(), "width": args.width,
                                                     "layers": args.layers, "minutes": elapsed / 60, "update": update,
                                                     "ema_updates": args.ema_updates})
        if isinstance(distill, DistillStream):
            distill.archive_used()

    def launch(actors, buffer: int, elapsed: float) -> dict:
        """Hand the idle actors the current weights and opponents, and start filling ``buffer``."""
        nonlocal next_pool
        if elapsed >= next_pool:
            pool.add(cpu_state(model), elapsed / 60)
            next_pool = elapsed + args.pool_minutes * 60
        progress = schedule_progress(args, elapsed)
        actors.set_weights("learner", cpu_state(model))
        snapshot = pool.sample(rng)  # PFSP; None until there are two snapshots
        if snapshot is not None and snapshot["id"] != loaded["snapshot"]:
            actors.set_weights("opponent", snapshot["state"])
            loaded["snapshot"] = snapshot["id"]
        opponents = np.where((kinds == SNAPSHOT) & (snapshot is None), SELF, kinds).astype(np.int8)
        ko_bonus = args.ko_bonus * max(0.0, 1 - progress / args.ko_bonus_end) if args.ko_bonus_end > 0 else 0.0
        teacher = teacher_coefficient(args, elapsed) > 0  # label only while the labels will still be trained on
        actors.start(buffer, opponents, ko_bonus, teacher=teacher, minutes=elapsed / 60)
        return dict(opponents=opponents, ko_bonus=ko_bonus, snapshot=None if snapshot is None else snapshot["id"],
                    snapshot_minutes=None if snapshot is None else snapshot["minutes"])

    with Actors(args.envs, args.steps, [args.pool], args.seed + update, args.workers, model,
                scripted=args.scripted, scripted_temperature=args.scripted_temperature,
                guards=MoveGuards(repeat_protect=not args.allow_repeat_protect), teacher=args.teacher_coef > 0,
                teacher_temperature=args.teacher_temperature,
                search=args.search_config if args.search_fraction > 0 else None,
                search_labels=args.search_labels if args.search_fraction > 0 else None) as actors:
        start = time.perf_counter() - elapsed_before
        current = 0
        info = launch(actors, current, elapsed_before)
        rollout = actors.wait()
        while True:
            elapsed = time.perf_counter() - start
            if (args.output / "STOP").exists() or elapsed >= stop:
                break
            if elapsed >= next_snapshot:
                name = f"m{int(round(next_snapshot / 60)):03d}.pt"
                save(args.output / "snapshots" / name,
                     {"model": cpu_state(model), "width": args.width, "layers": args.layers, "minutes": elapsed / 60,
                      "battles": battles, "update": update})
                if average is not None:
                    save(args.output / "ema" / name,
                         {"model": average.state_dict(), "width": args.width, "layers": args.layers,
                          "minutes": elapsed / 60, "battles": battles, "update": update,
                          "ema_updates": args.ema_updates})
                next_snapshot += args.snapshot_minutes * 60
            # The actors fill the other buffer with the current weights while the GPU trains on this one.
            collected, collected_info = rollout, info
            if collected["crashes"]:  # battles an engine panic aborted (as ties); the run goes on, the reports stay
                with (args.output / "crashes.jsonl").open("a", encoding="utf-8") as stream:
                    for crash in collected["crashes"]:
                        stream.write(json.dumps(crash) + "\n")
                print(f"warning: {len(collected['crashes'])} battle(s) aborted by an engine panic, see crashes.jsonl: "
                      f"{collected['crashes'][0]['message']}", flush=True)
            snapshot_envs = collected_info["opponents"] == SNAPSHOT  # PFSP: credit games begun against the snapshot
            pool.record(collected_info["snapshot"], float(collected["clean_games"][snapshot_envs].sum()),
                        float(collected["clean_score"][snapshot_envs].sum()))
            info = launch(actors, 1 - current, elapsed)
            progress = schedule_progress(args, elapsed)
            optimizer.lr = args.lr * (1 - (1 - args.lr_final) * progress)
            if lr_rule is not None:  # --auto-lr: on top of the schedule
                optimizer.lr *= lr_rule.lr_factor
            # Critic warm-up: the critic heads step with their own Adam, so the main optimizer's step count (and with
            # it the bias correction of the policy's fresh moments) starts when the policy starts learning.
            warming = update + 1 <= args.critic_warmup_updates
            if warming and critic_optimizer is None:
                critic_optimizer = Adam([p for name, p in model.named_parameters() if name.startswith(CRITIC_PREFIXES)],
                                        lr=optimizer.lr)
            if critic_optimizer is not None:
                critic_optimizer.lr = optimizer.lr
            if controller is not None:
                entropy_coef = controller.coef
            else:
                entropy_coef = args.entropy + (args.entropy_final - args.entropy) * progress
            teacher_coef = teacher_coefficient(args, elapsed)
            distill_coef = distill_coefficient(args, elapsed)
            probe = GradientNoise() if noise_scale is not None and noise_scale.due(update + 1) else None
            if probe is not None and probe_model is not None:
                probe_model.load_state_dict(average.state)
            t1 = time.perf_counter()
            model.train()
            stats, samples = ppo_update(model, critic_optimizer if warming else optimizer, actors.buffers[current].arrays,
                                        args, rng, entropy_coef, device, teacher_coef, distill, distill_coef, elapsed,
                                        update=update + 1, noise=probe,
                                        accumulate=lr_rule.accumulate if lr_rule is not None else 1,
                                        probe_model=probe_model)
            train_seconds = time.perf_counter() - t1
            noise_metrics = {}
            if noise_scale is not None:
                measured = probe.estimate() if probe is not None else None
                if measured is not None:
                    noise_scale.observe(measured["g2"], measured["trace"])
                    if lr_rule is not None:
                        lr_rule.update(noise_scale)
                raw = measured and measured["b_noise"]
                noise_metrics = {**noise_scale.summary(), "noise_b_update": None if raw is None else round(raw),
                                 **(lr_rule.summary() if lr_rule is not None else {})}
            if average is not None:
                average.update(model)
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
                **({"actor_teacher_s": round(collected["teacher_seconds"], 2)} if args.teacher_coef > 0 else {}),
                **({"actor_search_s": round(collected["search_seconds"], 2),
                    **{k: collected[k] for k in ("search_decisions", "search_forced", "search_cells", "search_labels",
                                                  "search_unswapped")}}
                   if args.search_fraction > 0 else {}),
                "mean_turns": round(collected["turns"] / max(collected["completed"], 1), 1),
                "guarded_slots": collected["guarded"], "aborted_battles": len(collected["crashes"]),
                **opponent_results(collected, collected_info), "opponent_minutes": collected_info["snapshot_minutes"],
                "lr": optimizer.lr, "progress": progress, "entropy_coef": round(entropy_coef, 5),
                "entropy_target": round(controller.target(progress), 3) if controller is not None else None,
                "ko_bonus": round(collected_info["ko_bonus"], 4), "pool_size": len(pool), **stats, **noise_metrics,
                **behaviour,
                "pfsp": pool.summary(), **memory_mb(),
                **({"mps_mb": torch.mps.driver_allocated_memory() >> 20} if device.type == "mps" else {}),
                **{k: v for k, v in collected.items() if k.startswith("actor_")},  # largest actor's memory
            }
            if distill is not None and update % args.distill_validate_every == 0:
                model.eval()
                metrics.update(validate(model, distill) or {})
            print(json.dumps(metrics), flush=True)
            with (args.output / "metrics.jsonl").open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(metrics) + "\n")
            if update % 10 == 0:
                checkpoint(time.perf_counter() - start)
    checkpoint(time.perf_counter() - start)
    print(f"Done: {update} updates, {battles} battles in {(time.perf_counter() - start) / 60:.1f} min", flush=True)


if __name__ == "__main__":
    main()
