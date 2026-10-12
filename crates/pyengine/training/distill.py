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

import os
from pathlib import Path
from zipfile import BadZipFile

import numpy as np
import torch

OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")
STATE_SHARED = ("tables",)


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
        data = {k: np.concatenate([s[k] for s in shards])
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
        weight = weight / weight.sum(1, keepdims=True)
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
    """Device-resident, use-once labels. Checkpoint masks refer to original shard rows; device tensors retain only
    unused rows. Completed names stay in the checkpoint even after archival, so a resent shard cannot be trained
    twice. Call ``archive_used`` only after saving the model and ``state_dict`` together in latest.pt."""

    def __init__(self, directory: Path, device, pairs: int = 4, holdout_rows: int = 4000, state: dict | None = None):
        self.directory, self.device, self.pairs = Path(directory), device, pairs
        self.holdout_rows = holdout_rows
        self.shards = {}
        self.used_rows = {}
        self.completed = set()
        self.pending_moves = set()
        self.used_total = 0
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
            raw = self._read(path)
            if raw is not None:
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
    def _read(path: Path) -> dict | None:
        try:
            with np.load(path, allow_pickle=False) as data:
                raw = {k: data[k] for k in data.files}
            for key in (*OBS_KEYS, "x", "pairs", "mask0", "mask1", "prior"):
                if len(raw[key]) != len(raw["x"]):
                    return None
            return raw
        except (OSError, ValueError, EOFError, KeyError, BadZipFile):
            # An interrupted transfer may have exposed a final name; retry it at the next update.
            return None

    def _resident(self, raw: dict) -> dict:
        data = self._prepare([raw])
        return {k: v if k == "_reference" else v.to(self.device) for k, v in data.items()}

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
                data = self._batch(data, np.arange(len(data["weight"]) - n, len(data["weight"])))
            keep[key] = (created, data)
            remaining -= n
        self._held_shards = keep
        self.held = {k: torch.cat([data[k] for _, data in keep.values()])
                     for k in next(iter(keep.values()))[1] if k != "_reference"}
        self.held_size = self.holdout_rows - remaining

    def scan(self) -> None:
        for path in sorted(self._files(self.directory)):
            name = path.name
            if name in self.completed:
                self.pending_moves.add(name)
                continue
            if name in self.shards:
                continue
            if name in self._held_seen:
                raw = None
                holdout = True
            else:
                raw = self._read(path)
                if raw is None:
                    continue
                holdout = bool(raw.get("holdout", False))
            if holdout:
                if raw is not None:
                    self._hold(name, raw)
                destination = self.directory / "heldout"
                destination.mkdir(parents=True, exist_ok=True)
                os.replace(path, destination / name)
                continue
            used = self.used_rows.get(name, np.zeros(len(raw["x"]), dtype=bool))
            if len(used) != len(raw["x"]):
                raise ValueError(f"Distill shard changed row count since checkpoint: {path}")
            self.used_rows[name] = used
            rows = np.flatnonzero(~used)
            if not len(rows):
                self.completed.add(name)
                self.pending_moves.add(name)
                self.used_rows.pop(name)
                continue
            sliced = {k: v[rows] for k, v in raw.items() if v.ndim > 0}
            self.shards[name] = dict(data=self._resident(sliced), rows=rows,
                                     teacher_minutes=float(raw.get("teacher_minutes", np.nan)))

    @property
    def size(self) -> int:
        return sum(len(s["rows"]) for s in self.shards.values())

    def describe(self) -> dict:
        return dict(train=self.size, held=self.held_size, pairs=self.pairs, stream=str(self.directory))

    def begin_update(self, rng, batch: int, max_batches: int, minibatches: int,
                     elapsed_minutes: float = 0.0) -> dict[int, dict]:
        """Scan once, then draw uniformly without replacement and spread full batches over the PPO minibatches."""
        self.scan()
        fresh = self.size
        count = min(fresh // batch, max_batches, minibatches)
        chosen = rng.choice(fresh, size=count * batch, replace=False).reshape(count, batch) if count else []
        names = list(self.shards)
        offsets = np.cumsum([0, *(len(self.shards[n]["rows"]) for n in names)])
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
            self.used_rows[name][shard["rows"][pick]] = True
            keep = np.ones(len(shard["rows"]), dtype=bool)
            keep[pick] = False
            if keep.any():
                shard["data"] = self._batch(shard["data"], np.flatnonzero(keep))
                shard["rows"] = shard["rows"][keep]
            else:
                self.completed.add(name)
                self.pending_moves.add(name)
                del self.shards[name], self.used_rows[name]
        used = count * batch
        self.used_total += used
        lag = elapsed_minutes - float(np.mean(teachers)) if teachers and np.isfinite(teachers).all() else None
        self.metrics = dict(distill_fresh=fresh, distill_used=used, distill_used_total=self.used_total,
                            distill_batches=count, distill_shards_pending=len(self.shards),
                            distill_teacher_lag_min=None if lag is None else round(lag, 3))
        return batches

    def state_dict(self) -> dict:
        return dict(used_rows={k: v.copy() for k, v in self.used_rows.items()}, completed=sorted(self.completed),
                    used_total=self.used_total)

    def load_state_dict(self, state: dict) -> None:
        self.used_rows = {k: np.asarray(v, dtype=bool).copy() for k, v in state.get("used_rows", {}).items()}
        self.completed = set(state.get("completed", []))
        self.used_total = int(state.get("used_total", 0))
        self.shards.clear()
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
