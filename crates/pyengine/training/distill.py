"""Search distillation (train_selfplay.py ``--distill``): imitate the search's strategy on labelled positions.

search_labels.py writes positions from search-vs-search games, each with the search's mixed strategy ``x`` over its
candidate pairs (slot a, slot b). The loss is the cross-entropy of the policy's pair log-probs against those weights,
``-sum_k x_k (lp0[a0_k] + lp1[a1_k | a0_k])``, over the ``pairs`` most weighted candidates of each position
(renormalized). Slot b is conditioned on each candidate's own slot a, so the trunk runs once per position and slot b's
head once per candidate. Masks are the guarded ones the policy plays with (stored by search_labels.py).

Shards whose index is ``holdout_every - 1`` modulo ``holdout_every`` are held out: ``validate`` reports their loss
and how often the policy's most likely candidate is the search's most weighted one.
"""
from __future__ import annotations

import math
import os
from pathlib import Path
from zipfile import ZipFile

import numpy as np
import torch

from .dex import load_dex
from .encoder import (I_CAND_ABILITIES, I_CAND_ITEMS, N_ACTION_FEATURES, N_FIELD_FLOATS, N_IDS,
                      N_POKEMON_FLOATS, N_TOKEN_FEATURES, N_TOKENS)
from .model import N_ACTIONS

OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")
STATE_SHARED = ("tables",)
ROW_KEYS = (*OBS_KEYS, "x", "pairs", "mask0", "mask1", "prior", "q", "value")
META_KEYS = ("teacher", "teacher_minutes", "created", "holdout")


def _take(state: dict, index: torch.Tensor) -> dict:
    return {k: (v if k in STATE_SHARED else v.index_select(0, index)) for k, v in state.items()}


class DistillData:
    def __init__(self, paths: list[Path], device, pairs: int = 4, holdout_every: int = 20):
        files = sorted(f for p in paths for f in (Path(p).glob("labels-*.npz") if Path(p).is_dir() else [Path(p)]))
        if not files:
            raise SystemExit(f"--distill: no labels-*.npz in {', '.join(map(str, paths))}")
        train, held = [], []
        for i, f in enumerate(files):
            with np.load(f) as data:
                (held if len(files) > 1 and i % holdout_every == holdout_every - 1 else train).append(
                    {k: data[k] for k in data.files})
        self.device = device
        self.pairs = pairs
        self.train = self._prepare(train)
        self.held = self._prepare(held) if held else None
        self.size = len(self.train["weight"])
        self.held_size = 0 if self.held is None else len(self.held["weight"])

    def _prepare(self, shards: list[dict]) -> dict:
        data = {k: shards[0][k] if len(shards) == 1 else np.concatenate([s[k] for s in shards])
                for k in (*OBS_KEYS, "x", "pairs", "mask0", "mask1", "prior")}
        x = data["x"].astype(np.float32)
        top = np.argsort(-x, axis=1, kind="stable")[:, :self.pairs]  # most weighted candidates first
        rows = np.arange(len(x))[:, None]
        weight = x[rows, top]
        pairs = data["pairs"][rows, top].astype(np.int64)
        mask1 = data["mask1"][rows, top]
        missing = pairs[..., 0] < 0  # fewer candidates than ``pairs``: repeat the first one with weight 0
        pairs[missing] = np.broadcast_to(pairs[:, :1], pairs.shape)[missing]
        mask1[missing] = np.broadcast_to(mask1[:, :1], mask1.shape)[missing]
        weight = np.where(missing, 0.0, weight)
        weight = weight / weight.sum(1, keepdims=True, dtype=np.float64)
        prior = np.where(missing, 0.0, data["prior"][rows, top])
        out = {k: data[k] for k in OBS_KEYS}
        out.update(mask0=data["mask0"], mask1=mask1, a0=pairs[..., 0], a1=pairs[..., 1], weight=weight.astype(np.float32))
        # Reference losses over the same candidates: the targets' own entropy (the floor) and the labelling policy's
        # cross-entropy (its prior's log-probs of the candidates).
        safe = np.where(weight > 0, weight, 1.0)
        self_entropy = float(-(weight * np.log(safe)).sum(1).mean())
        prior_ce = float(-(weight * prior).sum(1).mean())
        tensors = {k: torch.from_numpy(np.ascontiguousarray(v)) for k, v in out.items()}
        if self.device.type == "mps":  # unified memory: keep the set on the device and gather there
            tensors = {k: v.to(self.device) for k, v in tensors.items()}
        tensors["_reference"] = dict(target_entropy=round(self_entropy, 4), labeller_ce=round(prior_ce, 4))
        return tensors

    def describe(self) -> dict:
        return dict(train=self.size, held=self.held_size, pairs=self.pairs, **self.train["_reference"])

    def batch(self, index: np.ndarray, split: str = "train") -> dict:
        data = self.train if split == "train" else self.held
        return self._batch(data, index)

    def _batch(self, data: dict, index: np.ndarray) -> dict:
        index = torch.from_numpy(np.asarray(index, dtype=np.int64))
        if data["weight"].device.type != "cpu":
            index = index.to(self.device)
            return {k: v.index_select(0, index) for k, v in data.items() if k != "_reference"}
        return {k: v.index_select(0, index).to(self.device) for k, v in data.items() if k != "_reference"}

    def sample(self, rng, n: int) -> dict:
        return self.batch(rng.integers(0, self.size, size=n))


class DistillStream(DistillData):
    """Bounded label reuse, admitted newest first, with update gaps and age retirement. Only MPS caches rows on
    device; other devices receive sampled batches. Checkpoint row state refers to original shard rows, including
    rows waiting on disk. Completed names survive archival so resends cannot train. Save model and ``state_dict``
    before ``archive_used``."""

    def __init__(self, directory: Path, device, pairs: int = 4, holdout_rows: int = 4000, state: dict | None = None,
                 max_resident_rows: int = 60000, max_uses: int = 4, reuse_gap: int = 50,
                 max_age_minutes: float = 60.0, update: int = 0, elapsed_minutes: float = 0.0):
        if max_resident_rows <= 0:
            raise ValueError("max_resident_rows must be positive")
        if max_uses <= 0 or reuse_gap < 0 or math.isnan(max_age_minutes) or max_age_minutes < 0:
            raise ValueError("max_uses must be positive; reuse_gap and max_age_minutes must be nonnegative")
        self.directory, self.device, self.pairs = Path(directory), device, pairs
        self.holdout_rows, self.max_resident_rows = holdout_rows, max_resident_rows
        self.max_uses, self.reuse_gap, self.max_age_minutes = max_uses, reuse_gap, max_age_minutes
        self.update, self.elapsed_minutes = update, elapsed_minutes
        self.shards = {}
        self.row_state = {}
        self._catalog, self._reject_failed = {}, set()
        self.backlog_rows = 0
        self.completed = set()
        self.pending_moves = set()
        self.used_total = 0
        self.rows_spent_total = self.rows_retired_total = self.retired_unused_total = self.completed_uses_total = 0
        self.held = None
        self.held_size = 0
        self._held_shards, self._held_seen = {}, set()
        self.metrics = {}
        if state is not None:
            self.load_state_dict(state)
        # Names sort by process start time and sequence. Reload only enough newest heldout shards for validation.
        for path in sorted(self._files(self.directory / "heldout"), reverse=True):
            if self.held_size >= holdout_rows:
                break
            try:
                info = self._inspect(path)
                raw = self._read(path, np.arange(max(0, info["size"] - holdout_rows), info["size"]), info)
            except Exception as error:
                self._reject(path, error)
                continue
            self._hold(path.name, raw)

    @staticmethod
    def _files(directory: Path) -> list[Path]:
        try:
            with os.scandir(directory) as entries:
                return [Path(e.path) for e in entries
                        if e.name.startswith("labels-") and e.name.endswith(".npz") and e.is_file()]
        except FileNotFoundError:
            return []

    @staticmethod
    def _header(stream) -> tuple:
        version = np.lib.format.read_magic(stream)
        readers = {(1, 0): np.lib.format.read_array_header_1_0, (2, 0): np.lib.format.read_array_header_2_0}
        if version not in readers:
            raise ValueError(f"unsupported NPY version {version}")
        shape, fortran, dtype = readers[version](stream)
        return shape, fortran, dtype, stream.tell()

    @staticmethod
    def _inspect(path: Path) -> dict:
        """Read only NPY headers and scalar metadata, leaving backlog row arrays compressed on disk."""
        headers, metadata = {}, {}
        with ZipFile(path) as archive:
            members = archive.namelist()
            if len(members) != len(set(members)) or any(k not in {f"{key}.npy" for key in (*ROW_KEYS, *META_KEYS)}
                                                       for k in members):
                raise ValueError("unexpected or duplicate shard members")
            for key in ROW_KEYS:
                member = archive.getinfo(f"{key}.npy")
                with archive.open(member) as stream:
                    headers[key] = DistillStream._header(stream)
                shape, _, dtype, offset = headers[key]
                if dtype.hasobject or not dtype.isnative or member.file_size != offset + math.prod(shape) * dtype.itemsize:
                    raise ValueError(f"invalid {key} payload or dtype")
            shape = headers["x"][0]
            if len(shape) != 2 or not 0 < shape[1] <= N_ACTIONS ** 2:
                raise ValueError("x must have shape (rows, candidates)")
            n, candidates = shape
            shapes = dict(ids=(n, N_TOKENS, N_IDS), floats=(n, N_TOKENS, N_POKEMON_FLOATS),
                          field=(n, N_FIELD_FLOATS), action_features=(n, 2, 40, N_ACTION_FEATURES),
                          token_features=(n, N_TOKENS, N_TOKEN_FEATURES), x=shape, pairs=(n, candidates, 2),
                          mask0=(n, N_ACTIONS), mask1=(n, candidates, N_ACTIONS), prior=shape, q=shape, value=(n,))
            for key, expected in shapes.items():
                actual, _, dtype, _ = headers[key]
                valid_dtype = (dtype == np.dtype("int64") if key == "ids" else
                               dtype.kind == "i" and dtype.itemsize in (2, 4, 8) if key == "pairs" else
                               dtype == np.dtype("bool") if key in ("mask0", "mask1") else dtype == np.dtype("float32"))
                if actual != expected or not valid_dtype:
                    raise ValueError(f"invalid {key} shape or dtype: {actual}, {dtype}")
            present = {key for key in META_KEYS if f"{key}.npy" in members}
            if present and present != set(META_KEYS):
                raise ValueError("incomplete shard metadata")
            for key in present:
                with archive.open(f"{key}.npy") as stream:
                    shape, _, dtype, _ = DistillStream._header(stream)
                    valid_dtype = (dtype.kind == "U" and dtype.itemsize <= 4096 if key == "teacher" else
                                   dtype == np.dtype("bool") if key == "holdout" else dtype == np.dtype("float64"))
                    if shape != () or not valid_dtype:
                        raise ValueError(f"{key} must be scalar with the expected dtype")
                with archive.open(f"{key}.npy") as stream:
                    metadata[key] = np.lib.format.read_array(stream, allow_pickle=False)
            created, teacher = float(metadata.get("created", 0)), float(metadata.get("teacher_minutes", np.nan))
            if not np.isfinite(created) or created < 0 or np.isinf(teacher):
                raise ValueError("invalid shard timestamps")
        return dict(headers=headers, metadata=metadata, size=n)

    @staticmethod
    def _discard(stream, length: int) -> None:
        while length:
            block = stream.read(min(length, 65536))
            if not block:
                raise EOFError("truncated row data")
            length -= len(block)

    @staticmethod
    def _copy(stream, target: memoryview) -> None:
        cursor = 0
        while cursor < len(target):
            block = stream.read(min(len(target) - cursor, 65536))
            if not block:
                raise EOFError("truncated row data")
            target[cursor:cursor + len(block)] = block
            cursor += len(block)

    @staticmethod
    def _read(path: Path, rows: np.ndarray | None = None, info: dict | None = None) -> dict:
        """Decode just admitted rows, draining the remaining bytes in small blocks to check CRC/decompression.
        This also handles a shard larger than the entire cache without materializing its unused rows."""
        info = DistillStream._inspect(path) if info is None else info
        rows = np.arange(info["size"]) if rows is None else rows
        raw = dict(info["metadata"])
        with ZipFile(path) as archive:
            for key, header in info["headers"].items():
                shape, fortran, dtype, offset = header
                out = np.empty((len(rows), *shape[1:]), dtype=dtype, order="F" if fortran else "C")
                with archive.open(f"{key}.npy") as stream:
                    if DistillStream._header(stream) != header:
                        raise ValueError(f"{key} changed during admission")
                    width = math.prod(shape[1:])
                    # In Fortran order each feature column stores all rows consecutively.
                    columns = width if fortran else 1
                    row_bytes = dtype.itemsize if fortran else width * dtype.itemsize
                    target = memoryview(out.ravel(order="K")).cast("B")
                    cursor = 0
                    starts = np.r_[0, np.flatnonzero(np.diff(rows) != 1) + 1]
                    ends = np.r_[starts[1:], len(rows)]
                    for column in range(columns):
                        for begin, end in zip(starts, ends):
                            if begin == end:
                                continue
                            position = offset + (column * shape[0] + int(rows[begin])) * row_bytes
                            DistillStream._discard(stream, position - offset - cursor)
                            length = (end - begin) * row_bytes
                            start = (column * len(rows) + begin) * row_bytes
                            DistillStream._copy(stream, target[start:start + length])
                            cursor = position - offset + length
                    DistillStream._discard(stream, math.prod(shape) * dtype.itemsize - cursor)
                raw[key] = out
        DistillStream._validate_rows(raw)
        return raw

    @staticmethod
    def _validate_rows(raw: dict) -> None:
        for key in (*OBS_KEYS[1:], "x", "q", "prior", "value"):
            if not np.isfinite(raw[key]).all():
                raise ValueError(f"nonfinite {key}")
        x, pairs = raw["x"], raw["pairs"]
        valid = ((pairs >= 0) & (pairs < N_ACTIONS)).all(2)
        missing = (pairs == -1).all(2)
        if not (valid | missing).all() or (x < 0).any() or (x[missing] != 0).any() or (x.sum(1, dtype=np.float64) <= 0).any():
            raise ValueError("invalid candidate pairs or target weights; each x row needs a positive sum")
        row, candidate = np.nonzero(valid)
        if not (raw["mask0"][row, pairs[row, candidate, 0]] &
                raw["mask1"][row, candidate, pairs[row, candidate, 1]]).all():
            raise ValueError("candidate pair is masked out")
        dex = load_dex()
        limits = np.array([dex.n_species, dex.n_items, dex.n_abilities, *([dex.n_types] * 3),
                           *([dex.n_moves] * (I_CAND_ABILITIES - 6)),
                           *([dex.n_abilities] * (I_CAND_ITEMS - I_CAND_ABILITIES)),
                           *([dex.n_items] * (N_IDS - I_CAND_ITEMS))])
        if (raw["ids"] < 0).any() or (raw["ids"] >= limits).any():
            raise ValueError("observation ids outside embedding tables")

    def _reject(self, path: Path, error: Exception) -> None:
        self.shards.pop(path.name, None)
        self._catalog.pop(path.name, None)
        reason = " ".join(str(error).splitlines())
        try:
            destination = self.directory / "rejected"
            destination.mkdir(parents=True, exist_ok=True)
            os.replace(path, destination / path.name)
        except FileNotFoundError:
            return
        except OSError as move_error:
            self._reject_failed.add(path.name)
            reason += f"; quarantine failed: {move_error}"
        print(f"distill rejected {path.name}: {type(error).__name__}: {reason}", flush=True)

    @staticmethod
    def _select(data: dict, rows: np.ndarray) -> dict:
        index = torch.from_numpy(np.asarray(rows, dtype=np.int64)).to(data["weight"].device)
        return {k: v if k == "_reference" else v.index_select(0, index) for k, v in data.items()}

    def _resident(self, raw: dict) -> dict:
        return self._prepare([raw])

    def _hold(self, name: str, raw: dict) -> None:
        self._held_seen.add(name)
        if not self.holdout_rows or not len(raw["x"]):
            return
        n = min(len(raw["x"]), self.holdout_rows)
        rows = {k: v[-n:] for k, v in raw.items() if v.ndim > 0}
        self._held_shards[name] = (float(raw.get("created", 0)), self._resident(rows))
        keep, remaining = {}, self.holdout_rows
        for key in sorted(self._held_shards, key=lambda k: (self._held_shards[k][0], k), reverse=True):
            created, data = self._held_shards[key]
            n = min(len(data["weight"]), remaining)
            if not n:
                break
            if n < len(data["weight"]):
                data = self._select(data, np.arange(len(data["weight"]) - n, len(data["weight"])))
            keep[key] = (created, data)
            remaining -= n
        self._held_shards = keep
        self.held = {k: torch.cat([data[k] for _, data in keep.values()])
                     for k in next(iter(keep.values()))[1] if k != "_reference"}
        self.held_size = self.holdout_rows - remaining

    def scan(self) -> None:
        files = sorted(self._files(self.directory), reverse=True)
        present = {p.name for p in files}
        self._catalog = {name: info for name, info in self._catalog.items() if name in present}
        for path in files:
            name = path.name
            if name in self.completed:
                self.pending_moves.add(name)
                continue
            if name in self._reject_failed:
                continue
            if name in self._held_seen:
                destination = self.directory / "heldout"
                destination.mkdir(parents=True, exist_ok=True)
                os.replace(path, destination / name)
                continue
            try:
                info = self._catalog.get(name)
                if info is None:
                    info = self._catalog[name] = self._inspect(path)
                if bool(info["metadata"].get("holdout", False)):
                    rows = np.arange(max(0, info["size"] - self.holdout_rows), info["size"])
                    self._hold(name, self._read(path, rows, info))
                    destination = self.directory / "heldout"
                    destination.mkdir(parents=True, exist_ok=True)
                    os.replace(path, destination / name)
                    self._catalog.pop(name)
                    continue
                state = self.row_state.get(name)
                if state is not None and any(state[k].shape != (info["size"],) for k in ("uses", "last_used", "done")):
                    raise ValueError("shard changed row count since checkpoint")
                if state is not None:
                    self._retire(name, info)
            except Exception as error:
                self._reject(path, error)
                continue
        # Retire admitted shards before filling freed slots, even if a newer file appeared while the cache was full.
        for path in files:
            name = path.name
            if name not in self._catalog or name in self.completed or name in self._reject_failed:
                continue
            info = self._catalog[name]
            free = self.max_resident_rows - self.size
            if free <= 0:
                continue
            state = self.row_state.get(name)
            available = np.ones(info["size"], dtype=bool) if state is None else ~state["done"]
            if name in self.shards:
                available[self.shards[name]["rows"]] = False
            rows = np.flatnonzero(available)[:free]
            if not len(rows):
                if not info["size"]:
                    self.completed.add(name)
                    self.pending_moves.add(name)
                continue
            try:
                raw = self._read(path, rows, info)
            except Exception as error:
                self._reject(path, error)
                continue
            if state is None:
                self.row_state[name] = dict(uses=np.zeros(info["size"], dtype=np.int64),
                                            last_used=np.full(info["size"], -1, dtype=np.int64),
                                            done=np.zeros(info["size"], dtype=bool), admitted_minutes=self.elapsed_minutes)
            self._retire(name, info)
            if name in self.completed:
                continue
            data = self._resident(raw)
            del raw
            if name in self.shards:
                shard = self.shards[name]
                data = {k: v if k == "_reference" else torch.cat((shard["data"][k], v)) for k, v in data.items()}
                rows = np.r_[shard["rows"], rows]
            self.shards[name] = dict(data=data, rows=rows,
                                     teacher_minutes=float(info["metadata"].get("teacher_minutes", np.nan)))
        self.backlog_rows = sum(info["size"] - int(self.row_state[name]["done"].sum() if name in self.row_state else 0)
                                - (len(self.shards[name]["rows"]) if name in self.shards else 0)
                                for name, info in self._catalog.items() if name not in self.completed)

    def _prune(self, name: str) -> None:
        state = self.row_state[name]
        if name in self.shards:
            shard = self.shards[name]
            keep = np.flatnonzero(~state["done"][shard["rows"]])
            if not len(keep):
                del self.shards[name]
            elif len(keep) < len(shard["rows"]):
                shard["data"] = self._select(shard["data"], keep)
                shard["rows"] = shard["rows"][keep]
        if state["done"].all():
            self.completed.add(name)
            self.pending_moves.add(name)
            del self.row_state[name]

    def _retire(self, name: str, info: dict) -> None:
        state = self.row_state[name]
        teacher = float(info["metadata"].get("teacher_minutes", np.nan))
        origin = teacher if np.isfinite(teacher) else state["admitted_minutes"]
        if self.elapsed_minutes - origin > self.max_age_minutes:
            retiring = ~state["done"]
            self.rows_retired_total += int(retiring.sum())
            self.retired_unused_total += int((retiring & (state["uses"] == 0)).sum())
            self.completed_uses_total += int(state["uses"][retiring].sum())
            state["done"][retiring] = True
        self._prune(name)

    @property
    def size(self) -> int:
        return sum(len(s["rows"]) for s in self.shards.values())

    def describe(self) -> dict:
        return dict(train=self.size, held=self.held_size, pairs=self.pairs, stream=str(self.directory),
                    max_resident_rows=self.max_resident_rows, max_uses=self.max_uses, reuse_gap=self.reuse_gap,
                    max_age_minutes=self.max_age_minutes, distill_backlog_rows=self.backlog_rows)

    def begin_update(self, rng, batch: int, max_batches: int, minibatches: int,
                     elapsed_minutes: float = 0.0, update: int | None = None) -> dict[int, dict]:
        """Retire stale rows, admit up to the cap, and sample full batches without replacement, least-used first.
        Explicit update numbers follow the trainer; standalone callers advance the checkpointed counter."""
        self.update = self.update + 1 if update is None else update
        self.elapsed_minutes = elapsed_minutes
        self.scan()
        names = list(self.shards)
        offsets = np.cumsum([0, *(len(self.shards[n]["rows"]) for n in names)])
        uses = np.concatenate([self.row_state[n]["uses"][self.shards[n]["rows"]] for n in names]) if names else np.array([])
        last = np.concatenate([self.row_state[n]["last_used"][self.shards[n]["rows"]] for n in names]) if names else np.array([])
        eligible = (uses == 0) | ((last < self.update) & (self.update - last >= self.reuse_gap))
        available = int(eligible.sum())
        count = min(available // batch, max_batches, minibatches)
        chosen, remaining = [], count * batch
        if remaining:
            for tier in np.unique(uses[eligible]):
                rows = np.flatnonzero(eligible & (uses == tier))
                n = min(len(rows), remaining)
                chosen.extend(rng.choice(rows, size=n, replace=False))
                remaining -= n
                if not remaining:
                    break
        chosen = np.asarray(chosen, dtype=np.int64).reshape(count, batch)
        fresh_used = int((uses[chosen.ravel()] == 0).sum())
        taken = {name: [] for name in names}
        batches, teachers = {}, []
        positions = np.linspace(0, minibatches - 1, count, dtype=int) if count else []
        for position, pick in zip(positions, chosen):
            pieces = []
            for i, name in enumerate(names):
                local = pick[(pick >= offsets[i]) & (pick < offsets[i + 1])] - offsets[i]
                if not len(local):
                    continue
                shard = self.shards[name]
                pieces.append(self._batch(shard["data"], local))
                taken[name].extend(local)
                teachers.extend([shard["teacher_minutes"]] * len(local))
            batches[int(position)] = {k: torch.cat([piece[k] for piece in pieces]) for k in pieces[0]}
        for name, pick in taken.items():
            if not pick:
                continue
            shard = self.shards[name]
            state, rows = self.row_state[name], shard["rows"][pick]
            state["uses"][rows] += 1
            state["last_used"][rows] = self.update
            spent = rows[state["uses"][rows] >= self.max_uses]
            self.rows_spent_total += len(spent)
            self.completed_uses_total += int(state["uses"][spent].sum())
            state["done"][spent] = True
            self._prune(name)
        used = count * batch
        self.used_total += used
        lag = elapsed_minutes - float(np.mean(teachers)) if teachers and np.isfinite(teachers).all() else None
        completed = self.rows_spent_total + self.rows_retired_total
        self.metrics = dict(distill_eligible=available, distill_used=used, distill_used_fresh=fresh_used,
                            distill_used_total=self.used_total, distill_rows_spent_total=self.rows_spent_total,
                            distill_rows_retired_total=self.rows_retired_total,
                            distill_retired_unused_total=self.retired_unused_total,
                            distill_mean_uses_completed=self.completed_uses_total / completed if completed else None,
                            distill_backlog_rows=self.backlog_rows,
                            distill_batches=count, distill_shards_pending=len(self.shards),
                            distill_teacher_lag_min=None if lag is None else round(lag, 3))
        return batches

    def state_dict(self) -> dict:
        return dict(row_state={name: {k: v.copy() if isinstance(v, np.ndarray) else v for k, v in state.items()}
                               for name, state in self.row_state.items()}, completed=sorted(self.completed),
                    used_total=self.used_total, rows_spent_total=self.rows_spent_total,
                    rows_retired_total=self.rows_retired_total, retired_unused_total=self.retired_unused_total,
                    completed_uses_total=self.completed_uses_total, update=self.update, elapsed_minutes=self.elapsed_minutes)

    def load_state_dict(self, state: dict) -> None:
        self.update = int(state.get("update", self.update))
        self.elapsed_minutes = float(state.get("elapsed_minutes", self.elapsed_minutes))
        self.row_state = {name: dict(uses=np.asarray(s["uses"], dtype=np.int64).copy(),
                                    last_used=np.asarray(s["last_used"], dtype=np.int64).copy(),
                                    done=np.asarray(s["done"], dtype=bool).copy(), admitted_minutes=float(s["admitted_minutes"]))
                          for name, s in state.get("row_state", {}).items()}
        # A legacy use-once checkpoint's consumed rows stay complete, even when the new run allows reuse.
        for name, used in state.get("used_rows", {}).items():
            done = np.asarray(used, dtype=bool).copy()
            self.row_state[name] = dict(uses=done.astype(np.int64), last_used=np.full(len(done), -1, dtype=np.int64),
                                        done=done, admitted_minutes=self.elapsed_minutes)
        self.completed = set(state.get("completed", []))
        self.used_total = int(state.get("used_total", 0))
        self.rows_spent_total = int(state.get("rows_spent_total", self.used_total if "used_rows" in state else 0))
        self.rows_retired_total = int(state.get("rows_retired_total", 0))
        self.retired_unused_total = int(state.get("retired_unused_total", 0))
        self.completed_uses_total = int(state.get("completed_uses_total", self.rows_spent_total))
        self.shards.clear()
        self._catalog.clear()
        self.backlog_rows = 0
        self.pending_moves = set(self.completed)

    def archive_used(self) -> None:
        if self.pending_moves:
            destination = self.directory / "used"
            destination.mkdir(parents=True, exist_ok=True)
            for name in sorted(self.pending_moves):
                source = self.directory / name
                if source.exists():
                    os.replace(source, destination / name)
            self.pending_moves.clear()


def distill_loss(model, batch: dict) -> tuple[torch.Tensor, torch.Tensor]:
    """Mean over positions of ``-sum_k w_k log pi(a0_k, a1_k)``, and the share of positions whose most likely
    candidate under the policy is the most weighted one (detached)."""
    state = model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"], batch["token_features"])
    lp0, vectors0 = model.slot0(state, batch["mask0"])
    B, P = batch["a0"].shape
    a0, a1 = batch["a0"].reshape(-1), batch["a1"].reshape(-1)
    rows = torch.arange(B, device=a0.device).repeat_interleave(P)
    lp1 = model.slot1(_take(state, rows), vectors0.index_select(0, rows), a0, batch["mask1"].reshape(B * P, -1))
    n = lp0.shape[1]
    onehot0 = (a0.view(-1, 1) == torch.arange(n, device=a0.device)).float()
    onehot1 = (a1.view(-1, 1) == torch.arange(n, device=a1.device)).float()
    logp = ((lp0.index_select(0, rows) * onehot0).sum(-1) + (lp1 * onehot1).sum(-1)).view(B, P)
    weight = batch["weight"]
    loss = -(weight * logp).sum(1).mean()
    agree = (logp.detach().argmax(1) == weight.argmax(1)).float().mean()
    return loss, agree


@torch.no_grad()
def validate(model, data: DistillData, chunk: int = 512) -> dict | None:
    if data.held is None:
        return None
    total = agree = 0.0
    for begin in range(0, data.held_size, chunk):
        index = np.arange(begin, min(begin + chunk, data.held_size))
        loss, match = distill_loss(model, data.batch(index, "held"))
        total += float(loss) * len(index)
        agree += float(match) * len(index)
    return dict(distill_val_loss=round(total / data.held_size, 4), distill_val_agree=round(agree / data.held_size, 4))
