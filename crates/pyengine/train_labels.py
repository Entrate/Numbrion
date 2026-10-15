"""Train a policy only on search label shards (search_labels.py / --search-labels format), no self-play.

    python crates/pyengine/train_labels.py --labels DIR [DIR ...] --out OUT [--width 128 --layers 3]
        [--rows N] [--epochs 4] [--lr 1e-3] [--init CHECKPOINT] [--test DIR ...]

Every ``labels-*.npz`` under the ``--labels`` directories trains, except shards in ``heldout/`` folders: those (or
``--test``) are the test set. 2% of the training shards (whole shards, fixed seed) are held back for validation.
``--rows`` takes a random subset of whole shards. The learning rate decays from ``--lr`` to 0 by a cosine over
``--epochs`` passes and the final weights are the result (``OUT/model.pt``, the checkpoint format compare.py and
SearchBot load; the value and hidden-information heads stay untrained). Loss: training.distill.distill_loss over the
4 most weighted candidates (the --distill target).

Rows are kept compact in RAM (float16 features, int16 ids: the round trip changes features by < 5e-4). Sets up to
``--resident`` rows stay in RAM; larger ones stream through a shuffle buffer (each row once per epoch), so millions
of labels fit in a few GB.

Metrics (OUT/log.jsonl, OUT/result.json): ``agree`` = the policy's most likely of the search's top-4 candidates is
the search's top one; ``cond_ce`` = cross-entropy against the search's weights over them.

What it showed (docs/training/LABELS.md, 2026-10-15): a fresh 128x3 on 1.8M labels beat m535 52.7% policy-only;
192x4 learned no more than 128x3 at 1M labels; shallow search-opponent labels were worth as much as full-search ones.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import sys
import time
from pathlib import Path
from zipfile import ZipFile

import numpy as np
import torch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from training.device import training_device  # noqa: E402
from training.dex import load_dex  # noqa: E402
from training.distill import _take, distill_loss  # noqa: E402
from training.model import Model  # noqa: E402
from training.ppo import Adam, clip_gradients  # noqa: E402

OBS = ("ids", "floats", "field", "action_features", "token_features")
PAIRS = 4
BATCH = 256


def lower_priority():
    try:
        if sys.platform == "win32":
            import psutil
            psutil.Process().nice(psutil.BELOW_NORMAL_PRIORITY_CLASS)
        else:
            os.nice(5)
    except Exception as error:  # noqa: BLE001
        print("priority unchanged:", error, flush=True)


def rows_of(path: Path) -> int:
    with ZipFile(path) as z, z.open("x.npy") as s:
        version = np.lib.format.read_magic(s)
        read = np.lib.format.read_array_header_1_0 if version == (1, 0) else np.lib.format.read_array_header_2_0
        return int(read(s)[0][0])


def shards(directories: list[Path], heldout: bool | None) -> list[dict]:
    """Label shards under ``directories``: only those in heldout/ folders, only the others, or all (None)."""
    files = sorted({p for d in directories for p in Path(d).rglob("labels-*.npz")})
    return [dict(path=str(p), rows=rows_of(p)) for p in files if heldout is None or ("heldout" in p.parts) == heldout]


# ---- compact rows ----------------------------------------------------------------------------------------------------

def compact(path: str) -> dict:
    """One shard as compact arrays with the top-PAIRS candidates (DistillData._prepare's target)."""
    with np.load(path) as z:
        d = {k: z[k] for k in (*OBS, "x", "pairs", "mask0", "mask1")}
    x = d["x"].astype(np.float64)
    top = np.argsort(-x, axis=1, kind="stable")[:, :PAIRS]
    r = np.arange(len(x))[:, None]
    pairs = d["pairs"][r, top].astype(np.int16)
    mask1 = d["mask1"][r, top]
    missing = pairs[..., 0] < 0  # fewer candidates than PAIRS: repeat the first one with weight 0
    pairs[missing] = np.broadcast_to(pairs[:, :1], pairs.shape)[missing]
    mask1[missing] = np.broadcast_to(mask1[:, :1], mask1.shape)[missing]
    w = np.where(missing, 0.0, x[r, top])
    w = w / w.sum(1, keepdims=True)
    return dict(ids=d["ids"].astype(np.int16), floats=d["floats"].astype(np.float16),
                field=d["field"].astype(np.float16), action_features=d["action_features"].astype(np.float16),
                token_features=d["token_features"].astype(np.float16), mask0=d["mask0"], mask1=mask1,
                a0=pairs[..., 0], a1=pairs[..., 1], weight=w.astype(np.float32), valid=~missing)


def concat(parts: list[dict]) -> dict:
    return {k: np.concatenate([p[k] for p in parts]) for k in parts[0]}


def load_all(entries: list[dict]) -> dict:
    return concat([compact(e["path"]) for e in entries])


def to_batch(data: dict, index: np.ndarray, device) -> dict:
    out = {}
    for k in (*OBS, "mask0", "mask1", "a0", "a1", "weight", "valid"):
        v = data[k][index]
        if k in ("ids", "a0", "a1"):
            v = v.astype(np.int64)
        elif v.dtype == np.float16 or k == "valid":
            v = v.astype(np.float32)
        out[k] = torch.from_numpy(np.ascontiguousarray(v)).to(device)
    return out


class Stream:
    """Epochs over a list of shards: resident (all rows in RAM, reshuffled each epoch) or a shuffle buffer where the
    rows of each batch leave and the next shard's rows take their places (each row once per epoch)."""

    def __init__(self, entries: list[dict], resident: int, rng, buffer: int = 120_000):
        self.entries, self.rng = entries, rng
        self.rows = sum(e["rows"] for e in entries)
        self.resident = self.rows <= resident
        if self.resident:
            self.data = load_all(entries)
            self.order, self.pos = rng.permutation(len(self.data["weight"])), 0
            return
        self.queue, self.queue_pos, self.pending = [], 0, None
        parts, n = [], 0
        while n < buffer:
            parts.append(self._next_shard())
            n += len(parts[-1]["weight"])
        self.data = concat(parts)

    def _next_shard(self) -> dict:
        if self.queue_pos >= len(self.queue):
            self.queue, self.queue_pos = [self.entries[i] for i in self.rng.permutation(len(self.entries))], 0
        self.queue_pos += 1
        return compact(self.queue[self.queue_pos - 1]["path"])

    def batch_index(self) -> np.ndarray:
        if not self.resident:
            return self.rng.choice(len(self.data["weight"]), size=BATCH, replace=False)
        if self.pos + BATCH > len(self.order):
            self.order, self.pos = self.rng.permutation(len(self.order)), 0
        self.pos += BATCH
        return self.order[self.pos - BATCH:self.pos]

    def replace(self, index: np.ndarray):
        if self.resident:
            return
        while len(index):
            if self.pending is None or len(self.pending["weight"]) == 0:
                self.pending = self._next_shard()
            take = min(len(index), len(self.pending["weight"]))
            for k in self.data:
                self.data[k][index[:take]] = self.pending[k][:take]
            self.pending = {k: v[take:] for k, v in self.pending.items()}
            index = index[take:]


# ---- metrics ---------------------------------------------------------------------------------------------------------

@torch.no_grad()
def pair_logp(model, data: dict, device, chunk: int = 512) -> np.ndarray:
    """Log-probabilities of the stored candidates; fixed chunk shapes (DirectML caches kernels per shape)."""
    n = len(data["weight"])
    out = []
    for b in range(0, n, chunk):
        batch = to_batch(data, np.arange(b, b + chunk) % n, device)
        state = model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"],
                            batch["token_features"])
        lp0, vec0 = model.slot0(state, batch["mask0"])
        B, P = batch["a0"].shape
        a0, a1 = batch["a0"].reshape(-1), batch["a1"].reshape(-1)
        rows = torch.arange(B, device=a0.device).repeat_interleave(P)
        lp1 = model.slot1(_take(state, rows), vec0.index_select(0, rows), a0, batch["mask1"].reshape(B * P, -1))
        codes = torch.arange(lp0.shape[1], device=a0.device)
        lp = ((lp0.index_select(0, rows) * (a0.view(-1, 1) == codes).float()).sum(-1)
              + (lp1 * (a1.view(-1, 1) == codes).float()).sum(-1)).view(B, P)
        out.append(lp.double().cpu().numpy()[:min(chunk, n - b)])
    return np.concatenate(out)


def metrics(logp: np.ndarray, data: dict) -> dict:
    w, valid = data["weight"].astype(np.float64), data["valid"]
    lp = np.where(valid, logp, -np.inf)
    cond = lp - np.logaddexp.reduce(lp, axis=1, keepdims=True)
    ce = -(np.where(w > 0, w * np.where(valid, cond, 0.0), 0.0)).sum(1)
    hit = (cond.argmax(1) == w.argmax(1)).astype(np.float64)
    n = len(w)
    return dict(n=n, cond_ce=round(float(ce.mean()), 4), cond_ce_se=round(float(ce.std() / math.sqrt(n)), 4),
                agree=round(float(hit.mean()), 4), agree_se=round(float(hit.std() / math.sqrt(n)), 4))


# ---- training --------------------------------------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--labels", type=Path, nargs="+", required=True, help="label directories (searched recursively)")
    parser.add_argument("--test", type=Path, nargs="*", help="test directories (default: heldout/ under --labels)")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--rows", type=int, default=0, help="train on a random subset of about this many rows")
    parser.add_argument("--width", type=int, default=128)
    parser.add_argument("--layers", type=int, default=3)
    parser.add_argument("--init", type=Path, help="start from this checkpoint instead of a fresh model")
    parser.add_argument("--epochs", type=float, default=4.0)
    parser.add_argument("--lr", type=float, default=1e-3)
    parser.add_argument("--evals", type=int, default=12, help="validation points over the run")
    parser.add_argument("--resident", type=int, default=262_144, help="largest set kept fully in RAM")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--device", default="auto")
    args = parser.parse_args()

    lower_priority()
    args.out.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(args.seed)
    torch.manual_seed(args.seed)
    device, _ = training_device(args.device, 0)
    pool = shards(args.labels, heldout=False)
    test = shards(args.test, heldout=None) if args.test else shards(args.labels, heldout=True)
    if not pool:
        raise SystemExit(f"no labels-*.npz outside heldout/ in {args.labels}")
    pool = [pool[i] for i in rng.permutation(len(pool))]
    n_val = max(1, round(0.02 * len(pool)))
    val_entries, train_entries = pool[:n_val], pool[n_val:]
    if args.rows:
        total = np.cumsum([e["rows"] for e in train_entries])
        train_entries = train_entries[:int(np.searchsorted(total, args.rows)) + 1]

    dex = load_dex()
    if args.init:
        state = torch.load(args.init, map_location="cpu", weights_only=False)
        args.width, args.layers = state.get("width", 128), state.get("layers", 3)
        model = Model.from_state(dex, state["model"], args.width, args.layers)
    else:
        model = Model(dex, args.width, args.layers)
    model = model.to(device)
    optimizer = Adam(model.parameters(), lr=args.lr)
    stream = Stream(train_entries, args.resident, rng)
    val = load_all(val_entries)
    max_rows = int(args.epochs * stream.rows)
    every = max(BATCH, max_rows // max(1, args.evals))
    info = dict(train_rows=stream.rows, val_rows=len(val["weight"]), test_shards=len(test), resident=stream.resident,
                width=args.width, layers=args.layers, init=str(args.init) if args.init else None, lr=args.lr,
                epochs=args.epochs, batch=BATCH, device=str(device), params=sum(p.numel() for p in model.parameters()))
    print(json.dumps(info), flush=True)
    log = (args.out / "log.jsonl").open("a")
    seen, step, losses, t0 = 0, 0, [], time.time()
    while seen < max_rows:
        model.train()
        optimizer.lr = args.lr * 0.5 * (1 + math.cos(math.pi * seen / max_rows))
        index = stream.batch_index()
        loss, _ = distill_loss(model, to_batch(stream.data, index, device))
        optimizer.zero_grad()
        loss.backward()
        norm = clip_gradients(optimizer.parameters, 0.5)
        optimizer.step(skip=~torch.isfinite(norm))
        stream.replace(index)
        losses.append(float(loss))
        seen += BATCH
        step += 1
        if seen % every < BATCH or seen >= max_rows:
            model.eval()
            row = dict(step=step, rows_seen=seen, epochs=round(seen / stream.rows, 3), lr=round(optimizer.lr, 7),
                       train_loss=round(float(np.mean(losses)), 4), val=metrics(pair_logp(model, val, device), val),
                       minutes=round((time.time() - t0) / 60, 2))
            losses = []
            print(json.dumps(row), flush=True)
            log.write(json.dumps(row) + "\n")
            log.flush()
    del stream
    model.eval()
    torch.save({"model": {k: v.detach().cpu() for k, v in model.state_dict().items()}, "width": args.width,
                "layers": args.layers, "label_rows": info["train_rows"], "rows_seen": seen}, args.out / "model.pt")
    result = dict(info, minutes=round((time.time() - t0) / 60, 1))
    if test:
        data = load_all(test)
        result["test"] = metrics(pair_logp(model, data, device), data)
    (args.out / "result.json").write_text(json.dumps(result, indent=1))
    print("RESULT", json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
