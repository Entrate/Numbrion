"""Battles and observation encoding in worker processes; the policy runs in the main process.

Each worker owns a ``numbrion.BatchEnv`` slice and its ``Encoder`` and writes into shared-memory arrays that the
main process reads as one batch (rows ``2 * env + side``). A decision boundary takes two round trips:

1. main computes slot-a actions and calls ``mask_slot1(a0)``; workers answer with the conditional slot-b masks,
2. main computes slot-b actions and calls ``step(actions)``; workers step, auto-reset and encode the next boundary.

Python encoding is the expensive CPU part (training.encoder), so it runs in parallel across workers.
"""
from __future__ import annotations

import multiprocessing as mp
import sys
from multiprocessing import shared_memory
from pathlib import Path

import numpy as np

from .encoder import (N_ACTION_FEATURES, N_FIELD_FLOATS, N_IDS, N_POKEMON_FLOATS, N_TOKEN_FEATURES, N_TOKENS)

N_ACTIONS = 47


def layout(n_envs: int) -> dict:
    rows = 2 * n_envs
    return {
        "ids": ((rows, N_TOKENS, N_IDS), np.int64),
        "floats": ((rows, N_TOKENS, N_POKEMON_FLOATS), np.float32),
        "field": ((rows, N_FIELD_FLOATS), np.float32),
        "foe_match": ((rows, 6), np.int64),
        "action_features": ((rows, 2, 40, N_ACTION_FEATURES), np.float32),
        "token_features": ((rows, N_TOKENS, N_TOKEN_FEATURES), np.float32),
        "fainted": ((rows, 2), np.float32),
        "needs_action": ((n_envs, 2), np.bool_),
        "mask0": ((n_envs, 2, N_ACTIONS), np.bool_),
        "mask1": ((n_envs, 2, N_ACTIONS), np.bool_),
        "a0": ((n_envs, 2), np.int32),
        "actions": ((n_envs, 2, 2), np.int32),
        "reward": ((n_envs, 2), np.float32),
        "done": ((n_envs,), np.bool_),
        "winner": ((n_envs,), np.int8),
        "final_turns": ((n_envs,), np.int32),
        "battle_id": ((n_envs,), np.int64),
        "turn": ((n_envs,), np.int32),
    }


class Shared:
    def __init__(self, n_envs: int, names: dict | None = None):
        self.blocks, self.arrays = {}, {}
        for key, (shape, dtype) in layout(n_envs).items():
            size = int(np.prod(shape)) * np.dtype(dtype).itemsize
            if names is None:
                block = shared_memory.SharedMemory(create=True, size=max(size, 1))
            else:
                block = shared_memory.SharedMemory(name=names[key])
            self.blocks[key] = block
            self.arrays[key] = np.ndarray(shape, dtype=dtype, buffer=block.buf)
        if names is None:
            for array in self.arrays.values():
                array.fill(0)

    def names(self) -> dict:
        return {key: block.name for key, block in self.blocks.items()}

    def close(self, unlink: bool = False) -> None:
        self.arrays.clear()
        for block in self.blocks.values():
            block.close()
            if unlink:
                block.unlink()


def _worker(connection, names, n_envs, begin, end, pools, seed, threads, log_games, path):
    sys.path.insert(0, path)
    import torch

    import numbrion as nb

    torch.set_num_threads(1)

    from training.encoder import Encoder

    shared = Shared(n_envs, names)
    a = shared.arrays
    count = end - begin
    rows = slice(2 * begin, 2 * end)
    envs = slice(begin, end)
    env = nb.BatchEnv(count, [str(p) for p in pools], seed=seed, threads=threads, log=True)
    encoder = Encoder(2 * count)
    saved: list = []
    history = [[[], []] for _ in range(count)]

    def publish(result):
        a["needs_action"][envs] = result["needs_action"]
        a["mask0"][envs] = result["mask0"]
        a["reward"][envs] = result["reward"]
        a["done"][envs] = result["done"]
        a["winner"][envs] = result["winner"]
        a["final_turns"][envs] = result["final_turns"]
        a["battle_id"][envs] = result["battle_id"]
        a["turn"][envs] = result["turn"]
        observations = [env.observe(e, s) for e in range(count) for s in (0, 1)]
        if log_games:  # full player-view logs of finished battles, both sides
            for e in range(count):
                for side in (0, 1):
                    observation = observations[2 * e + side]
                    if result["done"][e]:
                        saved.append(dict(env=begin + e, side=side, winner=int(result["winner"][e]),
                                          battle=int(result["battle_id"][e]) - 1,
                                          log=history[e][side] + list(observation["prev_log"])))
                        history[e][side] = list(observation["log"])
                    else:
                        history[e][side].extend(observation["log"])
        b = encoder.encode_batch(observations)
        a["ids"][rows] = b.ids
        a["floats"][rows] = b.floats
        a["field"][rows] = b.field
        a["foe_match"][rows] = b.foe_match
        a["action_features"][rows] = b.action_features
        a["token_features"][rows] = b.token_features
        a["fainted"][rows] = b.fainted

    try:
        while True:
            command, argument = connection.recv()
            if command == "reset":
                publish(env.reset())
                connection.send(None)
            elif command == "mask1":
                a["mask1"][envs] = env.mask_slot1(np.ascontiguousarray(a["a0"][envs]))
                connection.send(None)
            elif command == "step":
                result = env.step(np.ascontiguousarray(a["actions"][envs]))
                if result["illegal"].any():
                    raise RuntimeError("A sampled action violated the exact mask")
                publish(result)
                connection.send(None)
            elif command == "random":
                a["actions"][envs] = env.random_actions(**argument)
                connection.send(None)
            elif command == "logs":
                connection.send(saved)
                saved = []
            elif command == "close":
                connection.send(None)
                break
    except Exception as error:  # surface worker failures to the main process
        import traceback
        connection.send(("error", traceback.format_exc()))
        raise error
    finally:
        shared.close()


class VecEnv:
    """``n_envs`` battles split over ``workers`` processes, each with ``threads`` Rust threads.

    ``mirror=True`` makes the second half of the battles a copy of the first half: the same worker seeds and env
    counts, so the k-th battle of env ``i`` and of env ``n_envs // 2 + i`` get the same teams and battle seed
    whatever is played (duplicate evaluation; team draws do not depend on actions).
    """

    def __init__(self, n_envs: int, pools: list[Path], seed: int, workers: int = 3, threads: int = 1,
                 log_games: bool = False, mirror: bool = False):
        self.n_envs = n_envs
        self.shared = Shared(n_envs)
        self.arrays = self.shared.arrays
        context = mp.get_context("spawn")
        if mirror:
            if n_envs % 2 or workers % 2:
                raise ValueError("mirror needs an even number of envs and workers")
            half = np.linspace(0, n_envs // 2, workers // 2 + 1).astype(int)
            bounds = np.concatenate((half, half[1:] + n_envs // 2))
            seeds = [seed * 1000 + w for w in range(workers // 2)] * 2
        else:
            bounds = np.linspace(0, n_envs, workers + 1).astype(int)
            seeds = [seed * 1000 + w for w in range(workers)]
        path = str(Path(__file__).resolve().parents[1])
        self.connections, self.processes = [], []
        for w in range(workers):
            parent, child = context.Pipe()
            process = context.Process(target=_worker, daemon=True, args=(
                child, self.shared.names(), n_envs, int(bounds[w]), int(bounds[w + 1]), pools,
                seeds[w], threads, log_games, path))
            process.start()
            self.connections.append(parent)
            self.processes.append(process)

    def _call(self, command: str, argument=None) -> list:
        for connection in self.connections:
            connection.send((command, argument))
        replies = [connection.recv() for connection in self.connections]
        for reply in replies:
            if isinstance(reply, tuple) and reply and reply[0] == "error":
                raise RuntimeError("worker failed:\n" + reply[1])
        return replies

    def reset(self) -> dict:
        self._call("reset")
        return self.arrays

    def mask_slot1(self, a0: np.ndarray) -> np.ndarray:
        self.arrays["a0"][:] = a0
        self._call("mask1")
        return self.arrays["mask1"]

    def step(self, actions: np.ndarray) -> dict:
        self.arrays["actions"][:] = actions
        self._call("step")
        return self.arrays

    def random_actions(self, switch_prob: float = 0.1, tera_prob: float = 0.15) -> np.ndarray:
        self._call("random", dict(switch_prob=switch_prob, tera_prob=tera_prob))
        return self.arrays["actions"].copy()

    def finished_logs(self) -> list:
        return [log for reply in self._call("logs") for log in reply]

    def close(self) -> None:
        try:
            self._call("close")
        except (BrokenPipeError, EOFError, OSError):
            pass
        for process in self.processes:
            process.join(timeout=5)
        self.shared.close(unlink=True)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
