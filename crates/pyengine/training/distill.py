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

from pathlib import Path

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
        data = {k: np.concatenate([s[k] for s in shards]) for k in shards[0]}
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
        index = torch.from_numpy(np.asarray(index, dtype=np.int64))
        if data["weight"].device.type != "cpu":
            index = index.to(self.device)
            return {k: v.index_select(0, index) for k, v in data.items() if k != "_reference"}
        return {k: v.index_select(0, index).to(self.device) for k, v in data.items() if k != "_reference"}

    def sample(self, rng, n: int) -> dict:
        return self.batch(rng.integers(0, self.size, size=n))


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
