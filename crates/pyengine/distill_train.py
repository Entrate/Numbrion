"""Train a policy and oracle critic entirely from search_labels.py shards.

    python distill_train.py train --labels LABEL_DIR --output STUDENT_DIR --device cpu
    python distill_train.py score STUDENT_DIR/latest.pt --labels LABEL_DIR

The default is a fresh 128x3, four epochs, Adam at 1e-3 with cosine decay to zero. Each row is used once per epoch.
Holdouts are whole shards, never sampled training rows. Checkpoints also support train_selfplay.py --resume.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import time
from pathlib import Path

import numpy as np
import torch

from training.device import BACKENDS, training_device
from training.dex import load_dex
from training.distill import OBS_KEYS, DistillData, DistillStream, _take, label_losses, value_target
from training.league import SnapshotPool
from training.model import Model
from training.ppo import Adam, clip_gradients


def files_in(paths):
    files = set()
    for path in paths:
        path = Path(path).resolve()
        if path.is_dir():
            for sub in (path, path / "used", path / "heldout"):
                files.update(sub.glob("labels-*.npz"))
        elif path.is_file():
            files.add(path)
        else:
            raise ValueError(f"label path does not exist: {path}")
    return sorted(files)


def split_shards(paths, heldout=(), holdout_every=20):
    files, explicit = files_in(paths), set(files_in(heldout))
    train, held = [], []
    for i, path in enumerate(files):
        info = DistillStream._inspect(path)
        is_held = (path in explicit or path.parent.name == "heldout" or
                   bool(info["metadata"].get("holdout", False)) or
                   ("holdout" not in info["metadata"] and i % holdout_every == holdout_every - 1))
        (held if is_held else train).append((path, info))
    held.extend((p, DistillStream._inspect(p)) for p in sorted(explicit - set(files)))
    if not train or not held:
        raise ValueError("need training and held-out shards; supply --heldout or generate at least 20 shards")
    if not sum(info["size"] for _, info in train) or not sum(info["size"] for _, info in held):
        raise ValueError("training and held-out splits must contain rows")
    return train, held


def compact(path, info, pairs=4, rows=None):
    raw = DistillStream._read(path, rows, info)
    loader = object.__new__(DistillData)
    loader.device, loader.pairs = torch.device("cpu"), pairs
    prepared = loader._prepare([raw])
    out = {}
    for key, value in prepared.items():
        if key == "_reference":
            continue
        array = value.numpy()
        if key in ("ids", "partner_ids", "foe_match", "a0", "a1"):
            array = array.astype(np.int16)
        elif key in (*OBS_KEYS[1:], "partner_floats"):
            array = array.astype(np.float16)
        out[key] = array
    return out


def concat(parts):
    return {k: np.concatenate([p[k] for p in parts]) for k in parts[0]}


def to_batch(data, index, device):
    out = {}
    for key, value in data.items():
        array = value[index]
        if array.dtype == np.int16:
            array = array.astype(np.int64)
        elif array.dtype == np.float16:
            array = array.astype(np.float32)
        out[key] = torch.from_numpy(np.ascontiguousarray(array)).to(device)
    return out


class Epochs:
    """Compact resident data, or bounded blocks of shuffled shards. No resampling or dropped tails."""

    def __init__(self, entries, pairs, resident_rows, buffer_rows):
        self.entries, self.pairs, self.buffer_rows = entries, pairs, buffer_rows
        self.rows = sum(info["size"] for _, info in entries)
        self.data = concat([compact(p, i, pairs) for p, i in entries]) if self.rows <= resident_rows else None

    def blocks(self, rng):
        if self.data is not None:
            yield self.data
            return
        parts, size = [], 0
        for index in rng.permutation(len(self.entries)):
            path, info = self.entries[index]
            for begin in range(0, info["size"], self.buffer_rows):
                rows = np.arange(begin, min(begin + self.buffer_rows, info["size"]))
                parts.append(compact(path, info, self.pairs, rows))
                size += len(rows)
                if size >= self.buffer_rows:
                    yield concat(parts)
                    parts, size = [], 0
        if parts:
            yield concat(parts)

    def batches(self, rng, batch_size, device):
        for block in self.blocks(rng):
            order = rng.permutation(len(block["weight"]))
            for begin in range(0, len(order), batch_size):
                yield to_batch(block, order[begin:begin + batch_size], device)


@torch.no_grad()
def score(model, entries, device, pairs=4, batch_size=256, value_mix=0.5, gamma=1.0):
    model.eval()
    n = hits = nv = 0
    ce = cond_ce = squared = target_sum = target_sq = error_sum = error_sq = 0.0
    for path, info in entries:
        # Keep validation bounded even if a user supplies one very large shard.
        for begin in range(0, info["size"], 4096):
            data = compact(path, info, pairs, np.arange(begin, min(begin + 4096, info["size"])))
            for b in range(0, len(data["weight"]), batch_size):
                batch = to_batch(data, slice(b, b + batch_size), device)
                state = model.trunk(*(batch[k] for k in OBS_KEYS))
                count = len(batch["weight"])
                # Same agreement as curve.py, plus its candidate-conditional CE.
                lp0, vec0 = model.slot0(state, batch["mask0"])
                B, P = batch["a0"].shape
                rows = torch.arange(B, device=device).repeat_interleave(P)
                a0, a1 = batch["a0"].flatten(), batch["a1"].flatten()
                lp1 = model.slot1(_take(state, rows), vec0[rows], a0, batch["mask1"].reshape(B * P, -1))
                logp = (lp0[rows].gather(1, a0[:, None]) + lp1.gather(1, a1[:, None])).view(B, P)
                conditional = logp.masked_fill(batch["valid"] == 0, -1e9).log_softmax(1)
                cond_ce += float(-(batch["weight"] * conditional).sum())
                n += count
                ce += float(-(batch["weight"] * logp).sum())
                hits += float((conditional.argmax(1) == batch["weight"].argmax(1)).float().sum())
                selected = torch.nonzero(batch["value_mask"], as_tuple=True)[0]
                if len(selected):
                    prediction = model.value(_take(state, selected), batch["partner_ids"][selected],
                                             batch["partner_floats"][selected])
                    target = value_target(batch, value_mix, gamma)[selected]
                    t = target.cpu().numpy().astype(np.float64)
                    error = (target - prediction).cpu().numpy().astype(np.float64)
                    nv += len(t)
                    target_sum += t.sum()
                    target_sq += (t * t).sum()
                    error_sum += error.sum()
                    error_sq += (error * error).sum()
                    squared += (error * error).sum()
    variance = target_sq / nv - (target_sum / nv) ** 2 if nv else 0
    error_variance = error_sq / nv - (error_sum / nv) ** 2 if nv else 0
    return dict(rows=n, policy_loss=ce / n, cond_ce=cond_ce / n, agree=hits / n, value_rows=nv,
                value_mse=float(squared / nv) if nv else None,
                value_explained_variance=float(1 - error_variance / variance) if variance > 1e-12 else None)


def save_checkpoint(path, model, optimizer, args, progress, rng, train, held):
    # PPO starts its own elapsed-time schedules and critic warmup at update zero. Adam moments are compatible
    # with training.ppo.Adam; label progress and RNGs live separately so --resume can continue label training.
    payload = dict(model={k: v.detach().cpu() for k, v in model.state_dict().items()},
                   width=model.width, layers=len(model.blocks), optimizer=optimizer.state_dict(),
                   pool=SnapshotPool().state_dict(), elapsed=0.0, update=0, battles=0, samples=0,
                   numpy_rng=np.random.default_rng(args.seed).bit_generator.state,
                   distill_training=dict(**progress, config=vars(args), numpy_rng=rng.bit_generator.state,
                                         torch_rng=torch.get_rng_state(), train=[str(p) for p, _ in train],
                                         heldout=[str(p) for p, _ in held]))
    temporary = path.with_suffix(".pt.tmp")
    torch.save(payload, temporary)
    os.replace(temporary, path)


def train(args):
    args.output.mkdir(parents=True, exist_ok=True)
    torch.manual_seed(args.seed)
    rng = np.random.default_rng(args.seed)
    device, adapter = training_device(args.device, args.adapter)
    source = args.resume or args.checkpoint
    state = torch.load(source, map_location="cpu", weights_only=False) if source else {}
    previous = state.get("distill_training", {})
    if args.resume:
        if not previous:
            raise ValueError("--resume needs a distill_train checkpoint; use --checkpoint to initialize from other models")
        # Keep the split fixed across resumes, even if new shards arrived in a source directory.
        train_entries = [(Path(p), DistillStream._inspect(Path(p))) for p in previous["train"]]
        held = [(Path(p), DistillStream._inspect(Path(p))) for p in previous["heldout"]]
        for key in ("pairs", "batch", "lr", "value_target_mix", "gamma", "value_coef", "aux_coef"):
            if getattr(args, key) != previous["config"][key]:
                raise ValueError(f"--resume must keep {key}={previous['config'][key]}")
    else:
        train_entries, held = split_shards(args.labels, args.heldout, args.holdout_every)
    dex = load_dex()
    model = (Model.from_state(dex, state["model"], state.get("width", 128), state.get("layers", 3)) if state else
             Model(dex, args.width, args.layers)).to(device)
    args.width, args.layers = model.width, len(model.blocks)
    optimizer = Adam(model.parameters(), lr=args.lr)
    epoch, seen = 0, 0
    if args.resume:
        optimizer.load_state_dict(state["optimizer"])
        rng.bit_generator.state = previous["numpy_rng"]
        torch.set_rng_state(previous["torch_rng"])
        epoch, seen = previous["epoch"], previous["rows_seen"]
    stream = Epochs(train_entries, args.pairs, args.resident_rows, args.shuffle_buffer)
    config = dict(vars(args), device=str(device), adapter_name=adapter, joint_features=model.joint_features,
                  train_rows=stream.rows,
                  train_shards=len(train_entries), heldout_shards=len(held),
                  train=[str(p) for p, _ in train_entries], held=[str(p) for p, _ in held])
    (args.output / "config.json").write_text(json.dumps(config, default=str, indent=2) + "\n")
    started = time.monotonic()
    metrics = score(model, held, device, args.pairs, args.batch, args.value_target_mix, args.gamma)
    print(json.dumps(dict(epoch=epoch, heldout=metrics)), flush=True)
    with (args.output / "metrics.jsonl").open("a") as log:
        while epoch < args.epochs:
            model.train()
            totals, supervised = np.zeros(4), np.zeros(2)
            for batch in stream.batches(rng, args.batch, device):
                # The last update uses lr=0, as in the fixed-budget cosine recipe.
                progress = min(seen / max(args.epochs * stream.rows - len(batch["weight"]), 1), 1.0)
                optimizer.lr = args.lr * 0.5 * (1 + math.cos(math.pi * progress))
                policy, value, aux, agree = label_losses(model, batch, args.value_target_mix, args.gamma)
                loss = policy + args.value_coef * value + args.aux_coef * aux
                optimizer.zero_grad()
                loss.backward()
                norm = clip_gradients(optimizer.parameters, 0.5)
                if not torch.isfinite(norm):
                    raise RuntimeError("nonfinite gradient; checkpoint remains at the previous completed epoch")
                optimizer.step()
                count = len(batch["weight"])
                counts = np.array([float(batch["value_mask"].sum()), float(batch["aux_mask"].sum())])
                terms = np.array([float(policy.detach()), float(value.detach()), float(aux.detach()), float(agree)])
                totals += terms * np.array([count, *counts, count])
                supervised += counts
                seen += count
            epoch += 1
            metrics = score(model, held, device, args.pairs, args.batch, args.value_target_mix, args.gamma)
            row = dict(epoch=epoch, rows_seen=seen, lr=optimizer.lr,
                       train=dict(zip(("policy_loss", "value_mse", "aux_loss", "agree"),
                                      (totals / np.array([stream.rows, *np.maximum(supervised, 1), stream.rows])).tolist())),
                       heldout=metrics, seconds=time.monotonic() - started)
            print(json.dumps(row), flush=True)
            log.write(json.dumps(row) + "\n")
            log.flush()
            save_checkpoint(args.output / "latest.pt", model, optimizer, args, dict(epoch=epoch, rows_seen=seen),
                            rng, train_entries, held)
    result = dict(epoch=epoch, rows_seen=seen, heldout=metrics, seconds=time.monotonic() - started)
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    training = commands.add_parser("train")
    scoring = commands.add_parser("score")
    scoring.add_argument("checkpoint", type=Path)
    for sub in (training, scoring):
        sub.add_argument("--labels", type=Path, nargs="+", default=[])
        sub.add_argument("--heldout", type=Path, nargs="+", default=[])
        sub.add_argument("--holdout-every", type=int, default=20)
        sub.add_argument("--pairs", type=int, default=4)
        sub.add_argument("--batch", type=int, default=256)
        sub.add_argument("--device", choices=BACKENDS, default="auto")
        sub.add_argument("--adapter", type=int, default=0)
        sub.add_argument("--torch-threads", type=int, default=1)
        sub.add_argument("--value-target-mix", type=float, default=0.5)
        sub.add_argument("--gamma", type=float, default=1.0,
                         help="per-decision discount of the outcome in the value target (1: pure win/loss; 0.995 was "
                              "the old PPO-matched default, which made a delayed loss worth more and rewarded stalling)")
    training.add_argument("--output", type=Path, required=True)
    source = training.add_mutually_exclusive_group()
    source.add_argument("--checkpoint", type=Path, help="initialize weights from a model, with a fresh optimizer")
    source.add_argument("--resume", type=Path, help="continue label training at the last completed epoch")
    training.add_argument("--width", type=int, default=128)
    training.add_argument("--layers", type=int, default=3)
    training.add_argument("--epochs", type=int, default=4)
    training.add_argument("--lr", type=float, default=1e-3)
    training.add_argument("--value-coef", type=float, default=0.5)
    training.add_argument("--aux-coef", type=float, default=0.03)
    training.add_argument("--seed", type=int, default=20261015)
    training.add_argument("--resident-rows", type=int, default=60000)
    training.add_argument("--shuffle-buffer", type=int, default=32768)
    args = parser.parse_args(argv)
    if (not 0 <= args.value_target_mix <= 1 or not 0 < args.gamma <= 1 or
            min(args.batch, args.pairs, args.torch_threads) < 1 or args.holdout_every < 2):
        parser.error("invalid mix, gamma, batch, pairs, threads or holdout interval")
    if args.command == "train":
        if not args.labels and not args.resume:
            parser.error("train needs --labels or --resume")
        if (args.epochs < 1 or args.width < 4 or args.width % 4 or args.layers < 1 or args.resident_rows < 0 or
                args.shuffle_buffer < 1 or not math.isfinite(args.lr) or args.lr <= 0 or
                not all(math.isfinite(x) and x >= 0 for x in (args.value_coef, args.aux_coef))):
            parser.error("invalid model, epoch, buffer or loss settings")
    return args


def main():
    args = parse_args()
    torch.set_num_threads(args.torch_threads)
    if args.command == "train":
        train(args)
    else:
        state = torch.load(args.checkpoint, map_location="cpu", weights_only=False)
        device, _ = training_device(args.device, args.adapter)
        model = Model.from_state(load_dex(), state["model"], state.get("width", 128), state.get("layers", 3)).to(device)
        if args.labels:
            _, held = split_shards(args.labels, args.heldout, args.holdout_every)
        else:
            paths = args.heldout or state.get("distill_training", {}).get("heldout", [])
            held = [(p, DistillStream._inspect(p)) for p in files_in(paths)]
            if not held:
                raise ValueError("score needs --labels, --heldout, or a checkpoint with saved held-out paths")
        print(json.dumps(score(model, held, device, args.pairs, args.batch, args.value_target_mix, args.gamma)), flush=True)


if __name__ == "__main__":
    main()
