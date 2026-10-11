"""Time-budgeted PPO self-play with the transformer policy (docs/training/TIPS.md, docs/training/SELFPLAY.md).

Run from the repository root:

    .venv\\Scripts\\python.exe crates\\pyengine\\train_selfplay.py --minutes 60

* Observations: training.encoder (own request + player-view protocol only), damage features (training.damage).
* Model: training.model (token transformer, per-action scoring, slot b conditioned on slot a, oracle critic,
  auxiliary hidden-information heads).
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
  trained on.
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
* Schedule: the learning rate anneals over the time budget; snapshots every ``--snapshot-minutes`` feed
  evaluate.py. ``latest.pt`` (with optimizer, opponent-pool and entropy-controller state) supports ``--resume``.
* Metrics: besides the losses, win rates against snapshots and against the heuristic, the PFSP table, and the
  learner's action usage (Tera, switches, Protect, status moves, Fake Out; training.diagnostics).
"""
from __future__ import annotations

import argparse
import json
import os
import time
from functools import lru_cache
from pathlib import Path

import numpy as np
import torch

from training.actors import Actors, memory_mb
from training.device import BACKENDS, training_device
from training.diagnostics import ActionUsage
from training.dex import load_dex
from training.guards import MoveGuards
from training.league import HEURISTIC, SELF, SNAPSHOT, SnapshotPool, opponent_kinds
from training.model import Model
from training.ppo import Adam, EntropyController, clip_gradients, gae, minibatches, tera_choice_entropy

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


def ppo_update(model, optimizer, data: dict, args, rng, entropy_coef: float, device,
               teacher_coef: float = 0.0) -> tuple[dict, int]:
    """PPO with the oracle critic and the auxiliary loss on one rollout buffer of [T, rows] arrays.

    With ``--teacher-coef`` set, the buffer carries teacher labels and the loss imitates them with weight
    ``teacher_coef`` (see ppo_loss). The term stays in the loss for the whole run, at weight 0 and without labels after
    the fade, so a compiled loss keeps one graph."""
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
    stats = {k: [] for k in (*terms_keys, "grad_norm")}
    # Loss coefficients as device tensors: the entropy coefficient changes every update, and DirectML keeps host
    # memory for every op given a new Python scalar (see training.ppo.Adam).
    dtype = next(model.parameters()).dtype
    value_coef, entropy_weight, aux_coef, tera_entropy_weight = (
        torch.full((), float(x), device=device, dtype=dtype)
        for x in (args.value_coef, entropy_coef, args.aux_coef, args.tera_entropy_weight))
    teacher_weight = torch.full((), float(teacher_coef), device=device, dtype=dtype) if teach else None
    loss_fn = compiled_ppo_loss() if getattr(args, "compile", False) else ppo_loss

    def up(array):
        return torch.from_numpy(np.ascontiguousarray(array)).to(device)

    # MPS shares memory with the CPU: copy the whole rollout once per update and gather the minibatches on the
    # device, instead of gathering them with NumPy and copying each (0.14 s of a 2.3 s update).
    resident = device.type == "mps"
    if resident:
        columns, weights = {k: up(v) for k, v in columns.items()}, up(weights)

    def rows(key, index, extra=()):
        return columns[key][(index, *extra)] if resident else up(columns[key][(index, *extra)])

    for _ in range(args.epochs):
        for pick in minibatches(rng.permutation(len(selected)), args.minibatch):
            index, partner_rows = selected[pick], partner_index[selected[pick]]
            if resident:
                pick, index, partner_rows = up(pick), up(index), up(partner_rows)
            batch = {k: rows(k, index) for k in OBS_KEYS}
            teacher = (dict(teacher_a0=rows("teacher_a0", index), teacher_a1=rows("teacher_a1", index),
                            teacher_labelled=rows("teacher_labelled", index), teacher_coef=teacher_weight)
                       if teach else {})
            loss, terms = loss_fn(model, batch, rows("ids", partner_rows), rows("floats", partner_rows, (slice(0, 6),)),
                                  rows("a0", index), rows("a1", index), rows("mask0", index), rows("mask1", index),
                                  rows("logp", index), weights[pick] if resident else up(weights[pick]),
                                  rows("returns", index), value_coef, entropy_weight, aux_coef, args.clip,
                                  tera_entropy_weight, **teacher)
            optimizer.zero_grad()
            loss.backward()
            norm = clip_gradients(optimizer.parameters, args.max_grad_norm)
            stats["grad_norm"].append(norm)
            optimizer.step(skip=~torch.isfinite(norm))  # a non-finite gradient leaves the weights as they were
            # Statistics stay on the device and are read back once per update (a .cpu() per value per minibatch
            # stalls the DirectML queue, ~10% of the update).
            for key, item in zip(terms_keys, terms):
                stats[key].append(item)
    stats = {k: torch.stack(v).cpu().numpy() if v else np.zeros(1) for k, v in stats.items()}
    if not np.isfinite(stats["policy_loss"]).all() or not np.isfinite(stats["grad_norm"]).all():
        raise RuntimeError("Non-finite PPO loss or gradient norm in this update (the optimizer skipped those steps, "
                           "so the weights are still finite)")
    values, rets = data["value"].reshape(-1)[selected], returns_all[selected]
    summary = {k: round(float(np.mean(v)), 4) for k, v in stats.items()}
    summary["explained_variance"] = round(float(1 - np.var(rets - values) / (np.var(rets) + 1e-8)), 3)
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


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--minutes", type=float, default=60.0, help="training wall-clock budget")
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
    parser.add_argument("--compile", action="store_true",
                        help="torch.compile the minibatch loss (~12 s of compiling per minibatch size at the start)")
    args = parser.parse_args()
    if args.teacher_coef < 0 or args.teacher_temperature < 0:
        parser.error("--teacher-coef and --teacher-temperature must be nonnegative")

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
        saved = state["model"]["act_damage.weight"].shape[1], state["model"]["float_proj.weight"].shape[1]
        if saved != (model.act_damage.in_features, model.float_proj.in_features):
            raise SystemExit(f"{args.resume} was trained on other damage-calc feature columns (training.damage); "
                             "start a new run (evaluate.py still plays it against new models)")
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
        teacher = teacher_coefficient(args, elapsed) > 0  # label only while the labels will still be trained on
        actors.start(buffer, opponents, ko_bonus, teacher=teacher)
        return dict(opponents=opponents, ko_bonus=ko_bonus, snapshot=None if snapshot is None else snapshot["id"],
                    snapshot_minutes=None if snapshot is None else snapshot["minutes"])

    with Actors(args.envs, args.steps, [args.pool], args.seed + update, args.workers, model,
                scripted=args.scripted, scripted_temperature=args.scripted_temperature,
                guards=MoveGuards(repeat_protect=not args.allow_repeat_protect), teacher=args.teacher_coef > 0,
                teacher_temperature=args.teacher_temperature) as actors:
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
            teacher_coef = teacher_coefficient(args, elapsed)
            t1 = time.perf_counter()
            model.train()
            stats, samples = ppo_update(model, optimizer, actors.buffers[current].arrays, args, rng, entropy_coef, device,
                                        teacher_coef)
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
                **({"actor_teacher_s": round(collected["teacher_seconds"], 2)} if args.teacher_coef > 0 else {}),
                "mean_turns": round(collected["turns"] / max(collected["completed"], 1), 1),
                "guarded_slots": collected["guarded"],
                **opponent_results(collected, collected_info), "opponent_minutes": collected_info["snapshot_minutes"],
                "lr": optimizer.lr, "entropy_coef": round(entropy_coef, 5),
                "entropy_target": round(controller.target(progress), 3) if controller is not None else None,
                "ko_bonus": round(collected_info["ko_bonus"], 4), "pool_size": len(pool), **stats, **behaviour,
                "pfsp": pool.summary(), **memory_mb(),
                **({"mps_mb": torch.mps.driver_allocated_memory() >> 20} if device.type == "mps" else {}),
                **{k: v for k, v in collected.items() if k.startswith("actor_")},  # largest actor's memory
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
