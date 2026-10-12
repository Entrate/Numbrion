"""Wrapper and worker integration checks that run without a native extension."""
import importlib
import sys
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest

from training import encoder, smart, vecenv


@pytest.mark.parametrize("force_python", (False, True))
@pytest.mark.parametrize("available", (False, True))
def test_backend_selection_and_missing_extension(monkeypatch, force_python, available):
    try:
        with monkeypatch.context() as patch:
            module = SimpleNamespace(ObsEncoder=object, SmartAgent=object) if available else SimpleNamespace()
            patch.setitem(sys.modules, "numbrion", module)
            patch.setenv("NUMBRION_PY_ENCODER", "1" if force_python else "0")
            patch.setenv("NUMBRION_PY_SMART", "1" if force_python else "0")
            importlib.reload(encoder)
            importlib.reload(smart)
            assert encoder.Encoder is (encoder.PyEncoder if force_python or not available else encoder.RustEncoder)
            assert smart.SmartHeuristicAgent is (smart.PySmartHeuristicAgent if force_python or not available else smart.RustSmartHeuristicAgent)
            patch.setitem(sys.modules, "numbrion", None)
            importlib.reload(encoder)
            importlib.reload(smart)
            assert encoder.Encoder is encoder.PyEncoder
            assert smart.SmartHeuristicAgent is smart.PySmartHeuristicAgent
    finally:
        importlib.reload(encoder)
        importlib.reload(smart)


@pytest.mark.parametrize("native_encoder", (False, True))
def test_package_exports_with_old_and_new_wheels(monkeypatch, native_encoder):
    name = "_numbrion_export_test"
    path = Path(__file__).resolve().parents[1] / "python" / "numbrion" / "__init__.py"
    spec = importlib.util.spec_from_file_location(name, path, submodule_search_locations=[str(path.parent)])
    package = importlib.util.module_from_spec(spec)
    exports = ("FORCED_MOVE", "N_ACTIONS", "N_MOVE_ACTIONS", "PASS", "SWITCH_BASE", "TARGET_AUTO", "BatchEnv",
               "Battle", "ChoiceError", "__version__", "decode_action", "describe_action", "move_action",
               "switch_action", "GridExecutor")
    native = SimpleNamespace(**{key: object for key in exports}, SEARCH=False)
    if native_encoder:
        native.ENCODER = True
        native.ObsEncoder = native.SmartAgent = object
    monkeypatch.setitem(sys.modules, name, package)
    monkeypatch.setitem(sys.modules, f"{name}._numbrion", native)
    monkeypatch.setitem(sys.modules, f"{name}.utils",
                        SimpleNamespace(grid_actions=object, sample_masked=object, sample_uniform=object))
    spec.loader.exec_module(package)
    assert package.ENCODER is native_encoder
    assert "ENCODER" in package.__all__
    for key in ("ObsEncoder", "SmartAgent"):
        assert hasattr(package, key) is native_encoder
        assert (key in package.__all__) is native_encoder
        if native_encoder:
            assert getattr(package, key) is getattr(native, key)


def test_encoder_forwards_buffers_in_place(monkeypatch):
    calls = []

    class Native:
        def __init__(self, path, rows):
            calls.append((path, rows))

        def encode_batch(self, observations, *arrays):
            calls.append(("batch", observations, arrays))
            arrays[0].fill(7)

        def encode_row(self, row, observation, *arrays):
            calls.append(("row", row, observation, arrays))

        def encode_env(self, env, *arrays, keep_logs=False):
            calls.append(("env", env, arrays, keep_logs))
            return [(["line"], ["prev"])] if keep_logs else None

    monkeypatch.setitem(sys.modules, "numbrion", SimpleNamespace(ObsEncoder=Native))
    dex = SimpleNamespace(path=Path("test-dex.json"))
    enc = encoder.RustEncoder(2, dex)
    observations = [dict(battle_id=1, request=None, log=[])] * 2
    assert enc.encode_batch(observations) is enc.buffers
    assert (enc.buffers.ids == 7).all()
    enc.encode(1, observations[1])
    env = object()
    assert enc.encode_env(env) is None
    assert enc.encode_env(env, keep_logs=True) == [(["line"], ["prev"])]
    assert enc.dex is dex
    assert calls[0] == ("test-dex.json", 2)
    expected = enc._outputs()
    assert len(expected) == 8
    for call in calls[1:]:
        arrays = call[3] if call[0] == "row" else call[2]
        assert all(a is b for a, b in zip(arrays, expected))


def test_smart_forwards_full_arrays_and_checks_second_rows(monkeypatch):
    calls = []

    class Native:
        def __init__(self, path, seed, **config):
            calls.append((path, seed, config))

        def first(self, *args):
            calls.append(args)
            return np.full(len(args[-2]), 46, dtype=np.int32)

        def second(self, mask):
            calls.append(mask)
            return np.full(len(mask), 46, dtype=np.int32)

    monkeypatch.setitem(sys.modules, "numbrion", SimpleNamespace(SmartAgent=Native))
    agent = smart.RustSmartHeuristicAgent(np.random.default_rng(4), SimpleNamespace(path=Path("dex.json")), 0.2)
    arrays = encoder.Buffers(6).arrays()
    rows = np.arange(6, dtype=np.int32)[::-2]
    mask = np.ones((3, 47), dtype=bool)
    assert agent.first(arrays, rows, mask).dtype == np.int32
    assert all(a is arrays[k] for a, k in zip(calls[1][:5], smart.OBS_KEYS))
    np.testing.assert_array_equal(calls[1][-2], rows)
    assert calls[1][-2].dtype == np.int64 and calls[1][-2].flags.c_contiguous
    assert calls[0][0] == "dex.json"
    assert calls[0][1] == int(np.random.default_rng(4).spawn(1)[0].integers(2**63))  # a spawned stream
    assert calls[0][2] == dict(temperature=0.2, noise=smart.NOISE, protect_skip=smart.PROTECT_SKIP,
                             protect_share=smart.PROTECT_SHARE)
    with pytest.raises(ValueError, match="same rows"):
        agent.second(rows[::-1], mask)
    assert agent.second(rows, mask).dtype == np.int32


@pytest.mark.parametrize("fast", (False, True))
@pytest.mark.parametrize("log_games", (False, True))
def test_worker_fast_path_preserves_finished_logs(monkeypatch, fast, log_games):
    arrays = {k: np.zeros(shape, dtype=dtype) for k, (shape, dtype) in vecenv.layout(3).items()}
    shared = SimpleNamespace(arrays=arrays, close=lambda: None)
    monkeypatch.setattr(vecenv, "Shared", lambda *args: shared)
    native_calls, observe_calls = [], []

    class Env:
        def __init__(self, *args, **kwargs):
            self.boundary = 0

        def result(self):
            out = {k: arrays[k][1:2].copy() for k in ("needs_action", "mask0", "reward", "done", "winner",
                                                     "final_turns", "battle_id", "turn")}
            out["done"][:] = self.boundary > 0
            out["battle_id"][:] = 10 + self.boundary
            out["winner"][:] = 1
            out["illegal"] = np.zeros((1, 2), dtype=bool)
            out["aborted"] = np.zeros(1, dtype=bool)
            return out

        def reset(self):
            return self.result()

        def step(self, actions):
            self.boundary += 1
            return self.result()

        def logs(self, side):
            return ([f"start{self.boundary}-{side}"], [f"end{self.boundary}-{side}"] if self.boundary else [])

        def observe(self, e, side):
            observe_calls.append((self.boundary, e, side))
            log, prev_log = self.logs(side)
            return dict(log=log, prev_log=prev_log)

    class Reference:
        def __init__(self, rows):
            self.buffers = encoder.Buffers(rows)

        def encode_batch(self, observations):
            assert len(observations) == 2
            return self.buffers

    class Fast(Reference):
        def encode_env(self, env, keep_logs=False):
            native_calls.append(keep_logs)
            return [env.logs(side) for side in (0, 1)] if keep_logs else None

    commands = iter((("reset", None), ("step", None), ("logs", None), ("step", None), ("logs", None), ("close", None)))
    sent = []
    connection = SimpleNamespace(recv=lambda: next(commands), send=sent.append)
    monkeypatch.setitem(sys.modules, "numbrion", SimpleNamespace(BatchEnv=Env))
    monkeypatch.setattr(encoder, "Encoder", Fast if fast else Reference)
    vecenv._worker(connection, {}, 3, 1, 2, [], 0, 1, log_games, str(Path(__file__).resolve().parents[1]))
    if fast:
        assert not observe_calls
        assert native_calls == [log_games] * 3
    else:
        assert len(observe_calls) == 6
    for boundary, logged in ((1, sent[2]), (2, sent[4])):
        if not log_games:
            assert logged == []
            continue
        assert logged == [dict(env=1, side=side, winner=1, battle=9 + boundary,
                               log=[f"start{boundary - 1}-{side}", f"end{boundary}-{side}"]) for side in (0, 1)]
