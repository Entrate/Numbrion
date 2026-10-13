"""Tests for the in-training search opponent (training.search_opponent): the batched matrix-game solve against
``training.search.solve``, the SEARCH opponent kind, and legal choices plus valid label shards on real battles."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import torch

import numbrion as nb
from training.dex import load_dex
from training.distill import DistillData, DistillStream
from training.encoder import Encoder
from training.guards import MoveGuards
from training.league import SEARCH, SELF, opponent_kinds
from training.model import Model
from training.search import solve, solve_batch
from training.search_opponent import LabelWriter, OpponentSearch, SearchOpponent

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"
OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")


def test_solve_batch_matches_solve():
    rng = np.random.default_rng(0)
    for _ in range(50):
        B = 4
        ns, ms = rng.integers(1, 5, B), rng.integers(1, 4, B)
        payoffs = rng.uniform(-1, 1, (B, ns.max(), ms.max()))
        ours, theirs = np.zeros((B, ns.max())), np.zeros((B, ms.max()))
        for b in range(B):
            ours[b, :ns[b]] = rng.dirichlet(np.ones(ns[b]))
            theirs[b, :ms[b]] = rng.dirichlet(np.ones(ms[b]))
        for anchor, opponent_anchor in ((0.05, 0.1), (0.05, 0.0), (0.0, 0.1)):
            x, q = solve_batch(payoffs, ours, theirs, anchor, opponent_anchor, 100)
            for b in range(B):
                n, m = ns[b], ms[b]
                xr, _, qr = solve([payoffs[b, :n, :m]], [np.ones(n, bool)], np.ones(1), ours[b, :n], [theirs[b, :m]],
                                  anchor, opponent_anchor, 100)
                assert np.allclose(x[b, :n], xr) and np.allclose(q[b, :n], qr)
                assert x[b, n:].sum() == 0


def test_search_kind_takes_self_play_envs():
    base = opponent_kinds(128, 0.3, 0.33)
    kinds = opponent_kinds(128, 0.3, 0.33, 0.2)
    assert (kinds == SEARCH).sum() == round(128 * 0.2)
    assert ((kinds != base) <= (base == SELF)).all()  # only self-play envs changed
    assert all((kinds[i * 32:(i + 1) * 32] == SEARCH).sum() >= 5 for i in range(4))  # spread over the actors


def test_search_opponent_plays_legal_pairs_and_writes_valid_labels(tmp_path):
    torch.manual_seed(0)
    dex = load_dex()
    model = Model(dex, 64, 1).eval()
    count = 6
    env = nb.BatchEnv(count, [str(POOL)], seed=11, threads=1, log=True)
    encoder = Encoder(2 * count)
    opponent = SearchOpponent(str(dex.path), MoveGuards(), OpponentSearch(iterations=50), seed=3,
                              labels=LabelWriter(tmp_path, size=32))
    result = env.reset()
    encoder.encode_env(env)
    b = encoder.buffers
    for _ in range(60):
        needs = result["needs_action"]
        envs = np.flatnonzero(needs[:, 1])
        with torch.no_grad():
            x = {k: torch.from_numpy(np.ascontiguousarray(getattr(b, k))) for k in OBS_KEYS}
            state = model.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
            choices = (opponent.act(model, env, encoder, state, {**b.arrays(), "guard": b.guard}, envs, needs, 2.0)
                       if len(envs) else {})
        assert set(choices) == set(envs.tolist())
        actions = env.random_actions(0.1, 0.1)
        for e, pair in choices.items():
            actions[e, 1] = pair
        actions[~needs] = -1
        result = env.step(actions)
        assert not result["illegal"].any()
        encoder.encode_env(env)
    opponent.labels.flush()
    assert opponent.stats["searched"] > 50
    files = sorted(tmp_path.glob("labels-*.npz"))
    assert files and sum(len(np.load(f)["x"]) for f in files) == opponent.labels.written == opponent.stats["searched"]
    for f in files:
        raw = dict(np.load(f))
        assert float(raw["teacher_minutes"]) == 2.0
        DistillStream._validate_rows(raw)
    assert DistillData([tmp_path], torch.device("cpu"), pairs=4).size == opponent.labels.written
