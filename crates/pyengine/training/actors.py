"""Actor processes that collect whole rollouts while the learner trains (one-update-stale PPO).

Each actor owns a ``numbrion.BatchEnv`` slice, its ``Encoder`` and CPU copies of the learner and the pool opponent.
It plays ``steps`` decision boundaries with the weights it was handed and writes observations, masks, actions,
behaviour log-probs, values, rewards and done flags into one of two shared-memory rollout buffers. The learner
trains on buffer ``k`` while the actors fill buffer ``1 - k``; PPO's ratio uses the stored behaviour log-probs, so
the data being one update old is accounted for. Weights travel through shared memory after every update.
"""
from __future__ import annotations

import multiprocessing as mp
import sys
import time
from multiprocessing import shared_memory
from pathlib import Path

import numpy as np

from .encoder import N_ACTION_FEATURES, N_FIELD_FLOATS, N_IDS, N_POKEMON_FLOATS, N_TOKEN_FEATURES, N_TOKENS

N_ACTIONS = 47
PASS = 46
OBS_KEYS = ("ids", "floats", "field", "foe_match", "action_features", "token_features")


def buffer_layout(steps: int, rows: int) -> dict:
    return {
        "ids": ((steps, rows, N_TOKENS, N_IDS), np.int64),
        "floats": ((steps, rows, N_TOKENS, N_POKEMON_FLOATS), np.float32),
        "field": ((steps, rows, N_FIELD_FLOATS), np.float32),
        "foe_match": ((steps, rows, 6), np.int64),
        "action_features": ((steps, rows, 2, 40, N_ACTION_FEATURES), np.float32),
        "token_features": ((steps, rows, N_TOKENS, N_TOKEN_FEATURES), np.float32),
        "mask0": ((steps, rows, N_ACTIONS), np.bool_),
        "mask1": ((steps, rows, N_ACTIONS), np.bool_),
        "a0": ((steps, rows), np.int64),
        "a1": ((steps, rows), np.int64),
        "logp": ((steps, rows), np.float32),
        "value": ((steps, rows), np.float32),
        "reward": ((steps, rows), np.float32),
        "done": ((steps, rows), np.bool_),
        "train": ((steps, rows), np.bool_),
        "bootstrap": ((rows,), np.float32),
    }


class SharedArrays:
    def __init__(self, layout: dict, names: dict | None = None):
        self.blocks, self.arrays = {}, {}
        for key, (shape, dtype) in layout.items():
            size = max(int(np.prod(shape)) * np.dtype(dtype).itemsize, 1)
            block = shared_memory.SharedMemory(create=names is None, size=size if names is None else 0,
                                               name=None if names is None else names[key])
            self.blocks[key] = block
            self.arrays[key] = np.ndarray(shape, dtype=dtype, buffer=block.buf)

    def names(self) -> dict:
        return {key: block.name for key, block in self.blocks.items()}

    def close(self, unlink: bool = False) -> None:
        self.arrays.clear()
        for block in self.blocks.values():
            block.close()
            if unlink:
                block.unlink()


def memory_mb() -> dict:
    """Working set and private bytes of this process (Windows), for leak checks."""
    if sys.platform != "win32":
        return {}
    import ctypes
    from ctypes import wintypes

    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in ("PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
                                                 "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
                                                 "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage",
                                                 "PrivateUsage")]

    kernel32 = ctypes.WinDLL("kernel32")
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE  # the pseudo-handle -1 must not be truncated to 32 bits
    kernel32.K32GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    counters = Counters()
    counters.cb = ctypes.sizeof(counters)
    if not kernel32.K32GetProcessMemoryInfo(kernel32.GetCurrentProcess(), ctypes.byref(counters), counters.cb):
        return {}
    return {"rss_mb": counters.WorkingSetSize >> 20, "private_mb": counters.PrivateUsage >> 20}


def weight_layout(model) -> list:
    """(name, shape, offset) of every persistent tensor in ``model.state_dict()``."""
    layout, offset = [], 0
    for name, tensor in model.state_dict().items():
        layout.append((name, tuple(tensor.shape), offset))
        offset += tensor.numel()
    return layout


def write_weights(flat: np.ndarray, layout: list, state: dict) -> None:
    for name, shape, offset in layout:
        size = int(np.prod(shape))
        flat[offset:offset + size] = state[name].detach().cpu().numpy().reshape(-1)


def read_weights(flat: np.ndarray, layout: list) -> dict:
    import torch

    return {name: torch.from_numpy(flat[offset:offset + int(np.prod(shape))].reshape(shape).copy())
            for name, shape, offset in layout}


def _actor(connection, buffer_names, weight_names, n_envs, steps, begin, end, pools, seed, path, model_config):
    sys.path.insert(0, path)
    import torch

    import numbrion as nb
    from training.dex import load_dex
    from training.encoder import Encoder
    from training.model import Model
    from training.ppo import sample

    torch.set_num_threads(1)
    R = 2 * n_envs
    buffers = [SharedArrays(buffer_layout(steps, R), names) for names in buffer_names]
    weights = SharedArrays({"learner": ((model_config["size"],), np.float32),
                            "opponent": ((model_config["size"],), np.float32)}, weight_names)
    dex = load_dex()
    learner = Model(dex, model_config["width"], model_config["layers"]).eval()
    opponent = Model(dex, model_config["width"], model_config["layers"]).eval()
    layout = weight_layout(learner)
    count = end - begin
    rows = slice(2 * begin, 2 * end)
    env = nb.BatchEnv(count, [str(p) for p in pools], seed=seed, threads=1, log=True)
    encoder = Encoder(2 * count)
    rng = np.random.default_rng(seed)
    versions = {"learner": -1, "opponent": -1}
    partner = np.arange(2 * count) ^ 1

    def observe(result):
        observations = [env.observe(e, s) for e in range(count) for s in (0, 1)]
        return encoder.encode_batch(observations)

    def tensors(b, index=slice(None)):
        return {k: torch.from_numpy(np.ascontiguousarray(getattr(b, k)[index])) for k in OBS_KEYS}

    result = env.reset()
    b = observe(result)
    try:
        while True:
            command, argument = connection.recv()
            if command == "close":
                connection.send(None)
                break
            # command == "collect"
            started = time.perf_counter()
            out = buffers[argument["buffer"]].arrays
            for key, model in (("learner", learner), ("opponent", opponent)):
                if argument[key + "_version"] != versions[key]:
                    model.load_state_dict(read_weights(weights.arrays[key], layout))
                    versions[key] = argument[key + "_version"]
            opp_rows = np.zeros(2 * count, dtype=bool)
            if argument["use_pool"]:
                opp_rows[1::2] = np.asarray(argument["pool_envs"][begin:end])
            opp_index = np.flatnonzero(opp_rows)
            ko_bonus = argument["ko_bonus"]
            stats = dict(completed=0, turns=0, pool_games=0, pool_wins=0)
            inference = 0.0
            for t in range(steps):
                for k in OBS_KEYS:
                    out[k][t, rows] = getattr(b, k)
                active = result["needs_action"].reshape(-1).copy()
                fainted_before = b.fainted.copy()
                mask0 = result["mask0"].reshape(-1, N_ACTIONS).copy()
                mask0[~active] = False
                mask0[~active, PASS] = True
                t0 = time.perf_counter()
                with torch.no_grad():
                    x = tensors(b)
                    state = learner.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
                    value = learner.value(state, x["ids"][partner], x["floats"][partner, 0:6]).numpy()
                    lp0_t, vectors0 = learner.slot0(state, torch.from_numpy(mask0))
                    lp0 = lp0_t.numpy().copy()
                    if len(opp_index):
                        y = tensors(b, opp_index)
                        opp_state = opponent.trunk(y["ids"], y["floats"], y["field"], y["action_features"], y["token_features"])
                        opp_lp0, opp_vectors0 = opponent.slot0(opp_state, torch.from_numpy(mask0[opp_index]))
                        lp0[opp_index] = opp_lp0.numpy()
                    a0 = sample(lp0, rng)
                    mask1 = env.mask_slot1(np.where(active, a0, -1).reshape(count, 2).astype(np.int32)).reshape(-1, N_ACTIONS).copy()
                    mask1[~active] = False
                    mask1[~active, PASS] = True
                    lp1 = learner.slot1(state, vectors0, torch.from_numpy(a0.astype(np.int64)), torch.from_numpy(mask1)).numpy().copy()
                    if len(opp_index):
                        lp1[opp_index] = opponent.slot1(opp_state, opp_vectors0, torch.from_numpy(a0[opp_index].astype(np.int64)),
                                                        torch.from_numpy(mask1[opp_index])).numpy()
                    a1 = sample(lp1, rng)
                inference += time.perf_counter() - t0
                actions = np.stack((a0, a1), -1).reshape(count, 2, 2).astype(np.int32)
                actions[~result["needs_action"]] = -1
                out["mask0"][t, rows], out["mask1"][t, rows] = mask0, mask1
                out["a0"][t, rows], out["a1"][t, rows] = a0, a1
                out["logp"][t, rows] = lp0[np.arange(len(a0)), a0] + lp1[np.arange(len(a1)), a1]
                out["value"][t, rows] = value
                out["train"][t, rows] = active & ~opp_rows
                result = env.step(actions)
                if result["illegal"].any():
                    raise RuntimeError("A sampled action violated the exact mask")
                b = observe(result)
                done = result["done"]
                reward = result["reward"].reshape(-1).astype(np.float32)
                if ko_bonus > 0:
                    delta = b.fainted - fainted_before
                    reward = reward + np.where(np.repeat(done, 2), 0.0, ko_bonus * (delta[:, 1] - delta[:, 0]))
                out["reward"][t, rows] = reward
                out["done"][t, rows] = np.repeat(done, 2)
                stats["completed"] += int(done.sum())
                stats["turns"] += int(result["final_turns"][done].sum())
                if argument["use_pool"]:
                    finished = done & opp_rows[1::2]
                    stats["pool_games"] += int(finished.sum())
                    stats["pool_wins"] += int((result["winner"][finished] == 0).sum())
            with torch.no_grad():
                x = tensors(b)
                state = learner.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
                out["bootstrap"][rows] = learner.value(state, x["ids"][partner], x["floats"][partner, 0:6]).numpy()
            stats["seconds"] = time.perf_counter() - started
            stats["inference_seconds"] = inference
            stats.update(memory_mb())
            connection.send(stats)
    except Exception:
        import traceback
        connection.send(("error", traceback.format_exc()))
        raise
    finally:
        for buffer in buffers:
            buffer.close()
        weights.close()


class Actors:
    """``n_envs`` battles over ``workers`` actor processes, two rollout buffers of ``steps`` boundaries."""

    def __init__(self, n_envs: int, steps: int, pools: list[Path], seed: int, workers: int, model):
        self.n_envs, self.steps = n_envs, steps
        self.buffers = [SharedArrays(buffer_layout(steps, 2 * n_envs)) for _ in range(2)]
        self.layout = weight_layout(model)
        size = sum(int(np.prod(shape)) for _, shape, _ in self.layout)
        self.weights = SharedArrays({"learner": ((size,), np.float32), "opponent": ((size,), np.float32)})
        self.versions = {"learner": 0, "opponent": 0}
        config = dict(size=size, width=model.width, layers=len(model.blocks))
        context = mp.get_context("spawn")
        bounds = np.linspace(0, n_envs, workers + 1).astype(int)
        path = str(Path(__file__).resolve().parents[1])
        self.connections, self.processes = [], []
        for w in range(workers):
            parent, child = context.Pipe()
            process = context.Process(target=_actor, daemon=True, args=(
                child, [b.names() for b in self.buffers], self.weights.names(), n_envs, steps, int(bounds[w]),
                int(bounds[w + 1]), pools, seed * 1000 + w, path, config))
            process.start()
            child.close()  # so recv() raises EOFError instead of hanging if the actor dies
            self.connections.append(parent)
            self.processes.append(process)
        self.pending = False

    def set_weights(self, key: str, state: dict) -> None:
        if self.pending:
            raise RuntimeError("weights may only change while the actors are idle")
        write_weights(self.weights.arrays[key], self.layout, state)
        self.versions[key] += 1

    def start(self, buffer: int, use_pool: bool, pool_envs: np.ndarray, ko_bonus: float) -> None:
        argument = dict(buffer=buffer, use_pool=use_pool, pool_envs=pool_envs.tolist(), ko_bonus=ko_bonus,
                        learner_version=self.versions["learner"], opponent_version=self.versions["opponent"])
        for connection in self.connections:
            connection.send(("collect", argument))
        self.pending = True

    def wait(self) -> dict:
        replies = [connection.recv() for connection in self.connections]
        self.pending = False
        for reply in replies:
            if isinstance(reply, tuple) and reply and reply[0] == "error":
                raise RuntimeError("actor failed:\n" + reply[1])
        totals = {k: sum(r[k] for r in replies) for k in ("completed", "turns", "pool_games", "pool_wins")}
        totals["seconds"] = max(r["seconds"] for r in replies)
        totals["inference_seconds"] = max(r["inference_seconds"] for r in replies)
        for key in ("rss_mb", "private_mb"):  # the largest actor
            if all(key in r for r in replies):
                totals["actor_" + key] = max(r[key] for r in replies)
        return totals

    def close(self) -> None:
        for connection in self.connections:
            try:
                connection.send(("close", None))
                connection.recv()
            except (BrokenPipeError, EOFError, OSError):
                pass
        for process in self.processes:
            process.join(timeout=5)
        for buffer in self.buffers:
            buffer.close(unlink=True)
        self.weights.close(unlink=True)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
