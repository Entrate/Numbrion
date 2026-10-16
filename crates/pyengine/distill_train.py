"""Train a policy and oracle critic entirely from search_labels.py shards.

    python distill_train.py train --labels LABEL_DIR --output STUDENT_DIR --device cpu
    python distill_train.py score STUDENT_DIR/latest.pt --labels LABEL_DIR

The default is a fresh 128x3, four epochs, Adam at 1e-3 with cosine decay to zero. Each row is used once per epoch.
Holdouts are whole shards, never sampled training rows. Checkpoints also support train_selfplay.py --resume.
``--init CHECKPOINT`` (alias ``--checkpoint``) fine-tunes existing weights with a fresh optimizer and schedule.

Label sets: ``--labels DIR[:weight[:value_mix[:max_rows]]] ...``. Plain paths (no suffix) form one set, exactly as
before (``--value-target-mix`` for its value targets). As soon as one entry has a suffix, every entry is its own
set: weight (default 1) is its share of the training stream, value_mix (default ``--value-target-mix``) the outcome
share of its value targets, max_rows (``4M``, ``500k``, ...) keeps a seeded random subset of its training shards
(and the same share of its held-out shards). Empty fields keep the default (``DIR::0.8``). Each set has its own
held-out shards; an epoch is the sum of the sets' training rows, and every batch draws its rows from the sets in
proportion to the weights (multinomial), from per-set shuffled streams that continue across epochs (a set whose
share exceeds its size repeats rows, a smaller share leaves some unused in an epoch). The streams' positions are
checkpointed, so ``--resume`` continues with the same rows. Sets may not share shard files (rejected), and no shard
is both trained on and held out. Held-out scores are reported per set, each with its own value mix. Rows marked
``pre_forced`` (sleep labels) always take their value target from the search value alone (training.distill).
"""
from __future__ import annotations

import argparse
import json
import math
import os
import re
import time
from dataclasses import dataclass, field
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


_SPEC_FIELD = re.compile(r"^(|\d+(\.\d*)?|\.\d+)([kKmM])?$")


@dataclass
class LabelSet:
    """One ``--labels`` entry (see the module docstring); ``train`` / ``held`` are (path, info) shard entries."""
    name: str
    paths: list
    weight: float = 1.0
    value_mix: float = 0.5
    max_rows: int | None = None
    train: list = field(default_factory=list)
    held: list = field(default_factory=list)

    def describe(self) -> dict:
        return dict(name=self.name, paths=[str(p) for p in self.paths], weight=self.weight, value_mix=self.value_mix,
                    max_rows=self.max_rows, train_rows=sum(i["size"] for _, i in self.train),
                    train_shards=len(self.train), heldout_shards=len(self.held))


def _count(text: str) -> int:
    scale = {"k": 1_000, "m": 1_000_000}.get(text[-1:].lower(), 1)
    return int(round(float(text[:-1] if scale > 1 else text) * scale))


def parse_label_spec(text: str) -> tuple[Path, list[str]]:
    """``DIR[:weight[:value_mix[:max_rows]]]`` -> (path, the given fields). An existing path is taken whole, so
    Windows drive letters and odd directory names still work."""
    if Path(text).exists():
        return Path(text), []
    parts = str(text).split(":")
    fields = []
    while len(parts) > 1 and len(fields) < 3 and _SPEC_FIELD.match(parts[-1]):
        fields.insert(0, parts.pop())
    return Path(":".join(parts)), fields


def label_sets(entries, default_mix: float) -> tuple[list[LabelSet], bool]:
    """The label sets of ``--labels`` and whether they are the legacy single set (no entry has a suffix)."""
    specs = [parse_label_spec(str(entry)) for entry in entries]
    if not any(fields for _, fields in specs):
        return [LabelSet("labels", [path for path, _ in specs], 1.0, default_mix)], True
    sets, names = [], set()
    for path, fields in specs:
        fields = fields + [""] * (3 - len(fields))
        name = path.stem if path.suffix == ".npz" else path.name or str(path)
        base, k = name, 1
        while name in names:
            k += 1
            name = f"{base}-{k}"
        names.add(name)
        sets.append(LabelSet(name, [path], float(fields[0]) if fields[0] else 1.0,
                             float(fields[1]) if fields[1] else default_mix,
                             _count(fields[2]) if fields[2] else None))
    for s in sets:
        if not (math.isfinite(s.weight) and s.weight >= 0 and 0 <= s.value_mix <= 1) or (
                s.max_rows is not None and s.max_rows < 1):
            raise ValueError(f"label set {s.name}: weight >= 0, value_mix in [0, 1], max_rows >= 1")
    if not sum(s.weight for s in sets) > 0:
        raise ValueError("label set weights must not all be zero")
    return sets, False


def cap_rows(train, held, max_rows, seed):
    """A seeded random subset of whole training shards with at least ``max_rows`` rows (all of them if fewer), and
    the same share of the held-out shards (at least one)."""
    total = sum(info["size"] for _, info in train)
    if max_rows is None or max_rows >= total:
        return train, held
    rng = np.random.default_rng(seed)
    kept, rows = [], 0
    for i in rng.permutation(len(train)):
        if rows >= max_rows:
            break
        kept.append(train[i])
        rows += train[i][1]["size"]
    count = max(1, math.ceil(len(held) * rows / max(total, 1)))
    return (sorted(kept, key=lambda e: e[0]),
            sorted([held[i] for i in rng.permutation(len(held))[:count]], key=lambda e: e[0]))


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


class SetStream:
    """One label set's endless shuffled row stream, pass after pass. Everything random derives from ``seed`` and the
    position, so the position ``(pass, block, cursor)`` is the whole sampling state (``state_dict``) and a resumed
    run continues the same rows. A pass visits the set's shards in a seeded order, cut into blocks of at least
    ``buffer_rows`` rows (all rows in one resident block when the set has at most ``resident_rows``), and each block
    is shuffled. Rows carry ids ``shard index << 32 | row``."""

    def __init__(self, entries, pairs, resident_rows, buffer_rows, seed):
        self.entries, self.pairs, self.buffer_rows, self.seed = entries, pairs, buffer_rows, list(seed)
        self.rows = sum(info["size"] for _, info in entries)
        self.resident = None
        if self.rows <= resident_rows:
            self.resident = concat([compact(p, i, pairs) for p, i in entries])
            self.resident_ids = np.concatenate([(np.int64(k) << 32) + np.arange(info["size"], dtype=np.int64)
                                                for k, (_, info) in enumerate(entries)])
        self.position = [0, 0, 0]  # pass, block, cursor
        self._layout = self._block = None

    def state_dict(self) -> dict:
        return dict(position=list(self.position))

    def load_state_dict(self, state: dict) -> None:
        self.position = [int(x) for x in state["position"]]
        self._layout = self._block = None

    def _blocks(self, number: int) -> list:
        """The (shard, begin, end) pieces of every block of pass ``number``."""
        if self._layout is not None and self._layout[0] == number:
            return self._layout[1]
        if self.resident is not None:
            blocks = [None]
        else:
            blocks, pieces, size = [], [], 0
            for k in np.random.default_rng([*self.seed, number, 0]).permutation(len(self.entries)):
                total = self.entries[k][1]["size"]
                for begin in range(0, total, self.buffer_rows):
                    end = min(begin + self.buffer_rows, total)
                    pieces.append((int(k), begin, end))
                    size += end - begin
                    if size >= self.buffer_rows:
                        blocks.append(pieces)
                        pieces, size = [], 0
            if pieces:
                blocks.append(pieces)
        self._layout = (number, blocks)
        return blocks

    def _load(self, number: int, block: int):
        if self._block is not None and self._block[0] == (number, block):
            return self._block[1:]
        pieces = self._blocks(number)[block]
        if pieces is None:
            data, ids = self.resident, self.resident_ids
        else:
            data = concat([compact(*self.entries[k], self.pairs, np.arange(b, e)) for k, b, e in pieces])
            ids = np.concatenate([(np.int64(k) << 32) + np.arange(b, e, dtype=np.int64) for k, b, e in pieces])
        order = np.random.default_rng([*self.seed, number, block + 1]).permutation(len(ids))
        self._block = ((number, block), data, ids, order)
        return data, ids, order

    def take(self, count: int) -> list:
        """The next ``count`` rows as compact parts, each with its row ids under ``_row``."""
        parts = []
        while count:
            number, block, cursor = self.position
            data, ids, order = self._load(number, block)
            if cursor >= len(order):
                block += 1
                if block >= len(self._blocks(number)):
                    number, block = number + 1, 0
                self.position = [number, block, 0]
                continue
            index = order[cursor:cursor + count]
            parts.append({**{k: v[index] for k, v in data.items()}, "_row": ids[index]})
            self.position[2] += len(index)
            count -= len(index)
        return parts


class MixedEpochs:
    """Label sets interleaved by weight (module docstring): every batch draws a multinomial number of rows from each
    set's SetStream (``rng``: the trainer's checkpointed generator), with a per-row ``value_mix`` column. An epoch is
    the sum of the sets' training rows. ``last_rows``: the row ids of the latest batch (set index << 48 | row id);
    ``state_dict`` / ``load_state_dict`` carry the streams' positions across a resume."""

    def __init__(self, sets, pairs, resident_rows, buffer_rows, seed=0):
        self.sets = [s for s in sets if s.weight > 0]
        self.streams = [SetStream(s.train, pairs, resident_rows, buffer_rows, [seed, i])
                        for i, s in enumerate(self.sets)]
        self.rows = sum(stream.rows for stream in self.streams)
        weights = np.array([s.weight for s in self.sets], dtype=np.float64)
        self.shares = weights / weights.sum()
        self.drawn = np.zeros(len(self.sets), dtype=np.int64)  # rows drawn per set (for reports and tests)
        self.last_rows = None

    def state_dict(self) -> dict:
        return dict(streams=[stream.state_dict() for stream in self.streams], drawn=self.drawn.tolist())

    def load_state_dict(self, state: dict) -> None:
        for stream, saved in zip(self.streams, state["streams"], strict=True):
            stream.load_state_dict(saved)
        self.drawn = np.asarray(state["drawn"], dtype=np.int64)

    def batches(self, rng, batch_size, device):
        produced = 0
        while produced < self.rows:
            n = min(batch_size, self.rows - produced)
            counts = rng.multinomial(n, self.shares)
            parts, rows = [], []
            for i, c in enumerate(counts):
                for part in self.streams[i].take(int(c)) if c else ():
                    rows.append((np.int64(i) << 48) + part.pop("_row"))
                    part["value_mix"] = np.full(len(rows[-1]), self.sets[i].value_mix, np.float32)
                    parts.append(part)
            self.drawn += counts
            produced += n
            self.last_rows = np.concatenate(rows)
            yield to_batch(concat(parts), slice(None), device)


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


def resolve_sets(args, previous):
    """(label sets, legacy) for a new run, or the saved split of a resumed one."""
    if args.resume:
        if "sets" in previous:
            sets = [LabelSet(s["name"], [Path(p) for p in s["paths"]], s["weight"], s["value_mix"], s.get("max_rows"),
                             [(Path(p), DistillStream._inspect(Path(p))) for p in s["train"]],
                             [(Path(p), DistillStream._inspect(Path(p))) for p in s["heldout"]])
                    for s in previous["sets"]]
            check_disjoint(sets)
            return sets, False
        # Keep the split fixed across resumes, even if new shards arrived in a source directory.
        legacy = LabelSet("labels", [], 1.0, args.value_target_mix,
                          train=[(Path(p), DistillStream._inspect(Path(p))) for p in previous["train"]],
                          held=[(Path(p), DistillStream._inspect(Path(p))) for p in previous["heldout"]])
        return [legacy], True
    sets, legacy = label_sets(args.labels, args.value_target_mix)
    if legacy:
        sets[0].train, sets[0].held = split_shards(sets[0].paths, args.heldout, args.holdout_every)
        return sets, True
    if args.heldout:
        raise ValueError("--heldout works with a single plain --labels set; label sets hold out their own shards")
    reject_overlaps(sets)
    for i, s in enumerate(sets):
        s.train, s.held = split_shards(s.paths, (), args.holdout_every)
        s.train, s.held = cap_rows(s.train, s.held, s.max_rows, args.seed + i)
    check_disjoint(sets)
    return sets, False


def reject_overlaps(sets) -> None:
    """Label sets must not share shard files (canonical paths): each set splits its own holdouts, so a shared file
    could be held out by one set and trained by another."""
    owner = {}
    for s in sets:
        for path in files_in(s.paths):
            identity = path.resolve()
            if identity in owner and owner[identity] != s.name:
                raise ValueError(f"label sets {owner[identity]} and {s.name} share shard {identity}; give each shard "
                                 "to one set only")
            owner[identity] = s.name


def check_disjoint(sets) -> None:
    """No shard is trained on by any set and held out by any set."""
    train = {path.resolve() for s in sets for path, _ in s.train}
    held = {path.resolve() for s in sets for path, _ in s.held}
    if train & held:
        raise ValueError(f"shards both trained on and held out: {sorted(map(str, train & held))[:3]}")


def score_sets(model, sets, legacy, device, args):
    """Held-out metrics: the legacy set's own dict, or one dict per label set (each with its value mix)."""
    if legacy:
        return score(model, sets[0].held, device, args.pairs, args.batch, args.value_target_mix, args.gamma)
    return {s.name: score(model, s.held, device, args.pairs, args.batch, s.value_mix, args.gamma) for s in sets}


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
            raise ValueError("--resume needs a distill_train checkpoint; use --init to initialize from other models")
        for key in ("pairs", "batch", "lr", "value_target_mix", "gamma", "value_coef", "aux_coef"):
            if getattr(args, key) != previous["config"][key]:
                raise ValueError(f"--resume must keep {key}={previous['config'][key]}")
    sets, legacy = resolve_sets(args, previous)
    train_entries = [entry for s in sets for entry in s.train]
    held = [entry for s in sets for entry in s.held]
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
    if legacy:
        stream = Epochs(train_entries, args.pairs, args.resident_rows, args.shuffle_buffer)
        progress_sets = {}
    else:
        stream = MixedEpochs(sets, args.pairs, args.resident_rows, args.shuffle_buffer, args.seed)
        if args.resume and "mixed_state" in previous:
            stream.load_state_dict(previous["mixed_state"])
        progress_sets = dict(sets=[dict(s.describe(), train=[str(p) for p, _ in s.train],
                                        heldout=[str(p) for p, _ in s.held]) for s in sets])
    config = dict(vars(args), device=str(device), adapter_name=adapter, joint_features=model.joint_features,
                  train_rows=stream.rows,
                  train_shards=len(train_entries), heldout_shards=len(held),
                  train=[str(p) for p, _ in train_entries], held=[str(p) for p, _ in held])
    if not legacy:
        config["sets"] = [s.describe() for s in sets]
        print("Label sets: " + json.dumps(config["sets"]), flush=True)
    (args.output / "config.json").write_text(json.dumps(config, default=str, indent=2) + "\n")
    started = time.monotonic()
    metrics = score_sets(model, sets, legacy, device, args)
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
            metrics = score_sets(model, sets, legacy, device, args)
            row = dict(epoch=epoch, rows_seen=seen, lr=optimizer.lr,
                       train=dict(zip(("policy_loss", "value_mse", "aux_loss", "agree"),
                                      (totals / np.array([stream.rows, *np.maximum(supervised, 1), stream.rows])).tolist())),
                       heldout=metrics, seconds=time.monotonic() - started)
            if not legacy:
                row["train"]["rows_drawn_per_set"] = dict(zip((s.name for s in stream.sets), stream.drawn.tolist()))
            print(json.dumps(row), flush=True)
            log.write(json.dumps(row) + "\n")
            log.flush()
            save_checkpoint(args.output / "latest.pt", model, optimizer, args,
                            dict(epoch=epoch, rows_seen=seen, **progress_sets,
                                 **({} if legacy else dict(mixed_state=stream.state_dict()))),
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
        sub.add_argument("--labels", nargs="+", default=[], metavar="DIR[:weight[:value_mix[:max_rows]]]",
                         help="label shards or directories; with a suffix, each entry is its own label set")
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
    source.add_argument("--init", "--checkpoint", dest="checkpoint", type=Path,
                        help="initialize weights from a model (fine-tuning), with a fresh optimizer and schedule")
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
    try:
        label_sets(args.labels, args.value_target_mix)
    except ValueError as error:
        parser.error(str(error))
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
            sets, legacy = label_sets(args.labels, args.value_target_mix)
            if not legacy:
                for s in sets:
                    s.held = split_shards(s.paths, (), args.holdout_every)[1]
                print(json.dumps({s.name: score(model, s.held, device, args.pairs, args.batch, s.value_mix, args.gamma)
                                  for s in sets}), flush=True)
                return
            _, held = split_shards(sets[0].paths, args.heldout, args.holdout_every)
        else:
            paths = args.heldout or state.get("distill_training", {}).get("heldout", [])
            held = [(p, DistillStream._inspect(p)) for p in files_in(paths)]
            if not held:
                raise ValueError("score needs --labels, --heldout, or a checkpoint with saved held-out paths")
        print(json.dumps(score(model, held, device, args.pairs, args.batch, args.value_target_mix, args.gamma)), flush=True)


if __name__ == "__main__":
    main()
