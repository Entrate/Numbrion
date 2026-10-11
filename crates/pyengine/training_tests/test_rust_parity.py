"""Native wheel checks: real player-view encoding, fast-path logs and deterministic scripted decisions."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

nb = pytest.importorskip("numbrion")
if not hasattr(nb, "ObsEncoder"):
    pytest.skip("wheel lacks the Rust encoder", allow_module_level=True)

from training import encoder as layout
from training import smart
from training.dex import load_dex
from training.encoder import Buffers, PyEncoder, RustEncoder

HERE = Path(__file__).resolve().parents[1]
POOL = HERE.parents[1] / "data" / "teams" / "train-s42-2000.txt"
if not POOL.exists():
    POOL = HERE / "testdata" / "pool-400.txt.gz"
OUTPUT_KEYS = ("ids", "floats", "field", "foe_match", "fainted", "guard", "action_features", "token_features")


def outputs(buffers):
    return tuple(getattr(buffers, name) for name in OUTPUT_KEYS)


def check_buffers(reference, actual):
    for name in OUTPUT_KEYS:
        expected, got = getattr(reference, name), getattr(actual, name)
        if expected.dtype.kind in "ib":
            np.testing.assert_array_equal(got, expected, err_msg=name)
        else:
            np.testing.assert_allclose(got, expected, atol=1e-5, rtol=1e-5, err_msg=name)


def make_env(n=4):
    return nb.BatchEnv(n, [str(POOL)], seed=827, threads=1, log=True)


def test_layout_matches_python():
    native = nb.ObsEncoder.layout()
    assert native["POKEMON_FLOAT_NAMES"] == layout.POKEMON_FLOATS
    assert native["FIELD_FLOAT_NAMES"] == layout.FIELD_FLOATS
    assert native["PF"] == layout.PF
    assert native["FF"] == layout.FF
    for name in ("VOLATILES", "TERRAINS", "SIDE_CONDITIONS", "TIMED_SIDE", "EXTREME_WEATHER", "STATUSES", "BOOSTS"):
        assert native[name] == list(getattr(layout, name)), name
    for name, value in vars(layout).items():
        if name.startswith(("N_", "I_", "K_", "LEGACY_")) or name in ("OWN", "FOE_A", "FOE_B", "STAT_SCALE", "CACHE_LIMIT"):
            assert native[name] == value, name


def test_selfplay_encoder_and_fast_path():
    torch.set_num_threads(1)
    n = 4
    env, fast_env, no_logs_env = make_env(n), make_env(n), make_env(n)
    result = env.reset()
    fast_env.reset()
    no_logs_env.reset()
    reference, native, fast, no_logs = PyEncoder(2 * n), RustEncoder(2 * n), RustEncoder(2 * n), RustEncoder(2 * n)
    finished = 0
    for boundary in range(256):
        observations = [env.observe(e, side) for e in range(n) for side in (0, 1)]
        reference.encode_batch(observations)
        native.encode_batch(observations)
        logs = fast.encode_env(fast_env, keep_logs=True)
        assert logs == [(o["log"], o["prev_log"]) for o in observations], boundary
        assert no_logs.encode_env(no_logs_env) is None
        for actual in (native, fast, no_logs):
            check_buffers(reference.buffers, actual.buffers)
        actions = env.random_actions()
        result = env.step(actions)
        for twin in (fast_env, no_logs_env):
            other = twin.step(actions)
            for key in ("battle_id", "done", "mask0", "needs_action"):
                np.testing.assert_array_equal(result[key], other[key])
        assert not result["illegal"].any()
        finished += int(result["done"].sum())
    assert finished > 0  # This also checks tracker reset and prev_log across auto-resets.


def test_encode_row_has_reference_semantics():
    torch.set_num_threads(1)
    env = make_env(1)
    env.reset()
    reference, native = PyEncoder(2), RustEncoder(2)
    for _ in range(12):
        for row in range(2):
            observation = env.observe(0, row)
            reference.encode(row, observation)
            native.encode(row, observation)
        check_buffers(reference.buffers, native.buffers)
        # Neither encode(row) implementation fills partner matching or damage features.
        assert (native.buffers.foe_match == -1).all()
        assert not native.buffers.action_features.any()
        assert not native.buffers.token_features.any()
        env.step(env.random_actions())


@pytest.mark.parametrize("name", OUTPUT_KEYS)
@pytest.mark.parametrize("invalid", ("dtype", "shape", "contiguity", "readonly"))
def test_output_validation(name, invalid):
    native = nb.ObsEncoder(str(load_dex().path), 2)
    b = Buffers(2)
    a = getattr(b, name)
    if invalid == "dtype":
        a = a.astype(np.float64)
    elif invalid == "shape":
        a = a[:1].copy()
    elif invalid == "contiguity":
        a = np.zeros((*a.shape[:-1], a.shape[-1] * 2), dtype=a.dtype)[..., ::2]
    else:
        a.flags.writeable = False
    setattr(b, name, a)
    observations = [dict(battle_id=0, request=None, log=[])] * 2
    with pytest.raises(ValueError, match=name):
        native.encode_batch(observations, *outputs(b))


def test_row_and_env_validation_before_draining():
    native = nb.ObsEncoder(str(load_dex().path), 2)
    b = Buffers(2)
    with pytest.raises(ValueError, match="observations"):
        native.encode_batch([], *outputs(b))
    with pytest.raises(ValueError, match="row"):
        native.encode_row(2, dict(battle_id=0, request=None, log=[]), *outputs(b))
    env = make_env(2)
    env.reset()
    with pytest.raises(ValueError, match="rows"):
        native.encode_env(env, *outputs(b))
    assert env.observe(0, 0)["log"]  # The failed call did not consume the observation.


def test_smart_deterministic_engine_masks(monkeypatch):
    torch.set_num_threads(1)
    monkeypatch.setattr(smart, "NOISE", 0.0)
    monkeypatch.setattr(smart, "PROTECT_SKIP", 0.0)
    monkeypatch.setattr(smart, "PROTECT_SHARE", (0.7, 0.7))
    n = 4
    env = make_env(n)
    result = env.reset()
    encoder = PyEncoder(2 * n)
    reference = smart.PySmartHeuristicAgent(np.random.default_rng(5))
    native = smart.RustSmartHeuristicAgent(np.random.default_rng(5))
    rng = np.random.default_rng(81)
    agreed, total = [0, 0], [0, 0]
    for _ in range(256):
        observations = [env.observe(e, side) for e in range(n) for side in (0, 1)]
        arrays = encoder.encode_batch(observations).arrays()
        rows = rng.permutation(np.flatnonzero(result["needs_action"].reshape(-1)))
        if len(rows):
            mask0 = result["mask0"].reshape(-1, 47)[rows]
            py0 = reference.first(arrays, rows, mask0)
            rs0 = native.first(arrays, rows.astype(np.int32), mask0)
            a0 = np.full(2 * n, nb.PASS, dtype=np.int32)
            a0[rows] = py0
            mask1 = env.mask_slot1(a0.reshape(n, 2)).reshape(-1, 47)[rows]
            py1 = reference.second(rows, mask1)
            a0[rows] = rs0
            rs_mask1 = env.mask_slot1(a0.reshape(n, 2)).reshape(-1, 47)[rows]
            rs1 = native.second(rows, rs_mask1)
            for slot, (expected, got, mask) in enumerate(((py0, rs0, mask0), (py1, rs1, rs_mask1))):
                assert got.dtype == np.int32
                assert mask[np.arange(len(rows)), got].all()
                agreed[slot] += int((expected == got).sum())
                total[slot] += len(rows)
        result = env.step(env.random_actions())
    for slot in (0, 1):
        assert total[slot] > 500
        assert agreed[slot] / total[slot] >= 0.995, (slot, agreed[slot], total[slot])


def test_smart_input_validation():
    native = nb.SmartAgent(str(load_dex().path), 0, noise=0, protect_skip=0, protect_share=(0.7, 0.7))
    b = Buffers(2)
    args = tuple(getattr(b, k) for k in smart.OBS_KEYS)
    rows, mask = np.array([0], dtype=np.int64), np.zeros((1, 47), dtype=bool)
    mask[:, nb.PASS] = True
    with pytest.raises(ValueError, match="first"):
        native.second(mask)
    with pytest.raises(ValueError, match="rows"):
        native.first(*args, rows.astype(np.int32), mask)
    with pytest.raises(ValueError, match="row index"):
        native.first(*args, np.array([2], dtype=np.int64), mask)
    with pytest.raises(ValueError, match="mask0"):
        native.first(*args, rows, mask[:, :46].copy())
    native.first(*args, rows, mask)
    with pytest.raises(ValueError, match="mask1"):
        native.second(np.zeros((2, 47), dtype=bool))
