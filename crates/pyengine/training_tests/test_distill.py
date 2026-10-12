"""Tests for search distillation (training.distill): label loading, the pair cross-entropy against a direct
per-pair computation, padding, and that the loss can be learned."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import torch

import numbrion as nb
from training.dex import load_dex
from training.distill import DistillData, distill_loss
from training.encoder import Encoder
from training.model import Model

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"
N_ACTIONS, K, N_ENVS = 47, 8, 8
OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")


def synthetic_labels(directory: Path, shards=3, seed=5):
    """Label shards in search_labels.py's format from random-play boundaries: up to K legal candidate pairs per
    position (some positions have fewer, padded with -1) with random weights."""
    rng = np.random.default_rng(seed)
    env = nb.BatchEnv(N_ENVS, [str(POOL)], seed=seed, threads=1, log=True)
    encoder = Encoder(2 * N_ENVS)
    result = env.reset()
    for shard in range(shards):
        rows_out = []
        while len(rows_out) < 24:
            arrays = encoder.encode_batch([env.observe(e, s) for e in range(N_ENVS) for s in (0, 1)]).arrays()
            mask0_all = result["mask0"].reshape(-1, N_ACTIONS)
            for row in np.flatnonzero(result["needs_action"].reshape(-1)):
                firsts = np.flatnonzero(mask0_all[row])
                pairs, mask1 = [], []
                for a0 in rng.permutation(firsts)[:4]:
                    joint = np.full(2 * N_ENVS, -1, dtype=np.int32)
                    joint[row] = a0
                    m1 = env.mask_slot1(joint.reshape(N_ENVS, 2)).reshape(-1, N_ACTIONS)[row].copy()
                    for a1 in rng.permutation(np.flatnonzero(m1))[:2]:
                        pairs.append((a0, a1))
                        mask1.append(m1)
                n = min(len(pairs), int(rng.integers(1, K + 1)))
                padded = np.full((K, 2), -1, dtype=np.int16)
                padded[:n] = pairs[:n]
                m1 = np.zeros((K, N_ACTIONS), dtype=bool)
                m1[:n] = mask1[:n]
                x = np.zeros(K, dtype=np.float32)
                x[:n] = rng.dirichlet(np.ones(n))
                label = {k: arrays[k][row].copy() for k in OBS_KEYS}
                label.update(mask0=mask0_all[row].copy(), mask1=m1, pairs=padded, x=x, q=np.zeros(K, np.float32),
                             prior=np.full(K, -2.0, np.float32), value=np.float32(0))
                rows_out.append(label)
            result = env.step(env.random_actions(switch_prob=0.15, tera_prob=0.2))
        np.savez_compressed(directory / f"labels-{shard:04d}.npz",
                            **{k: np.stack([r[k] for r in rows_out]) for k in rows_out[0]})


def model_for_test(seed=0):
    torch.manual_seed(seed)
    return Model(load_dex(), 32, 1)


def test_loss_matches_direct_pair_log_probs(tmp_path):
    synthetic_labels(tmp_path)
    data = DistillData([tmp_path], torch.device("cpu"), pairs=K, holdout_every=3)
    assert data.size > 0 and data.held_size > 0
    model = model_for_test()
    batch = data.batch(np.arange(min(16, data.size)))
    loss, _ = distill_loss(model, batch)
    expected = []
    with torch.no_grad():
        for i in range(len(batch["weight"])):
            one = {k: v[i:i + 1] for k, v in batch.items()}
            state = model.trunk(*(one[k] for k in OBS_KEYS))
            lp0, vectors0 = model.slot0(state, one["mask0"])
            total = 0.0
            for p in range(K):
                w = float(one["weight"][0, p])
                if w == 0:
                    continue
                a0, a1 = one["a0"][:, p], one["a1"][:, p]
                lp1 = model.slot1(state, vectors0, a0, one["mask1"][:, p])
                total -= w * float(lp0[0, a0] + lp1[0, a1])
            expected.append(total)
    assert abs(float(loss.detach()) - float(np.mean(expected))) < 1e-4


def test_padding_and_top_pairs(tmp_path):
    """Missing candidates repeat the first with weight 0; ``pairs`` keeps the most weighted, renormalized."""
    synthetic_labels(tmp_path, shards=1)
    raw = np.load(tmp_path / "labels-0000.npz")
    data = DistillData([tmp_path], torch.device("cpu"), pairs=3)
    weight, a0 = data.train["weight"].numpy(), data.train["a0"].numpy()
    assert np.allclose(weight.sum(1), 1)
    n = (raw["pairs"][:, :, 0] >= 0).sum(1)
    for i in range(len(n)):
        top = np.sort(raw["x"][i])[::-1][:3]
        expect = top[top > 0] / top[top > 0].sum()
        assert np.allclose(weight[i][:len(expect)], expect, atol=1e-6)
        assert (weight[i][len(expect):] == 0).all() and (a0[i][len(expect):] == a0[i][0]).all()


def test_loss_can_be_learned(tmp_path):
    synthetic_labels(tmp_path, shards=1)
    data = DistillData([tmp_path], torch.device("cpu"), pairs=K)
    model = model_for_test(1)
    optimizer = torch.optim.Adam(model.parameters(), lr=3e-3)
    batch = data.batch(np.arange(data.size))
    first, _ = distill_loss(model, batch)
    for _ in range(60):
        loss, _ = distill_loss(model, batch)
        optimizer.zero_grad()
        loss.backward()
        optimizer.step()
    last, agree = distill_loss(model, batch)
    assert float(last) < 0.7 * float(first)
    assert float(agree) > 0.5
