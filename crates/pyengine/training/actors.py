"""Actor processes that collect whole rollouts while the learner trains (one-update-stale PPO).

Each actor owns a ``numbrion.BatchEnv`` slice, its ``Encoder`` and CPU copies of the learner and the pool opponent.
It plays ``steps`` decision boundaries with the weights it was handed and writes observations, masks, actions,
behaviour log-probs, values, rewards and done flags into one of two shared-memory rollout buffers. The learner
trains on buffer ``k`` while the actors fill buffer ``1 - k``; PPO's ratio uses the stored behaviour log-probs, so
the data being one update old is accounted for. Weights travel through shared memory after every update.
Side 2 of each env is played by the learner, the pool snapshot or the scripted heuristic (training.league); the
actors report per-env results so the trainer can attribute them to the opponent it assigned.

Teacher labels (``Actors(teacher=True)``, rollouts started with ``teacher=True``): a separate smart player
(training.smart) labels every decision the learner trains on, DAgger-style, on the learner's own states:
``teacher_a0`` is its slot-a choice under the learner's guarded slot-a mask, ``teacher_a1`` its slot-b choice for the
learner's sampled slot-a code under the learner's guarded slot-b mask (the same slot-a code the learner's slot-b
log-probs are conditioned on), -1 for rows without a label. The labels change nothing the learner plays or samples.

Search opponent (``Actors(search=...)``): side 2 of SEARCH envs is played by shallow one-turn search with the learner's
weights on the env's true battle (training.search_opponent), which can write its decisions as distillation labels.
``search_value``: a fixed checkpoint (any size) each actor loads once for the search's leaf values instead of the
learner's value head (no weight sync: it never changes). ``search_both``: side 1 of SEARCH envs searches too (each
side hides the other's unseen bench); ``search_explore``: the share of searched decisions that play the learner's own
sample at temperature 1 instead (the label is still the search's). shallow_labels.py uses both, with every env SEARCH.
"""
from __future__ import annotations

import multiprocessing as mp
import sys
import time
from dataclasses import asdict
from functools import lru_cache
from multiprocessing import shared_memory
from pathlib import Path

import numpy as np

from .encoder import N_ACTION_FEATURES, N_FIELD_FLOATS, N_IDS, N_POKEMON_FLOATS, N_TOKEN_FEATURES, N_TOKENS
from .guards import MoveGuards

N_ACTIONS = 47
PASS = 46
OBS_KEYS = ("ids", "floats", "field", "foe_match", "action_features", "token_features")
RESULT_KEYS = ("games", "score", "clean_games", "clean_score", "battle_turns")  # per-env opponent results


def buffer_layout(steps: int, rows: int, teacher: bool = False) -> dict:
    """Rollout arrays; ``teacher`` adds the teacher labels (see the module docstring)."""
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
        **({"teacher_a0": ((steps, rows), np.int64), "teacher_a1": ((steps, rows), np.int64)} if teacher else {}),
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


@lru_cache(maxsize=1)
def _process_memory_api():
    """``K32GetProcessMemoryInfo`` set up once: ctypes caches a pointer type per Structure class forever."""
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
    return kernel32, Counters


@lru_cache(maxsize=1)
def _darwin_memory_api():
    """``proc_pid_rusage`` (libproc) with ``rusage_info_v2``: a uuid, then 18 uint64 counters."""
    import ctypes

    class RusageInfoV2(ctypes.Structure):
        _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [(name, ctypes.c_uint64) for name in (
            "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups", "pageins", "wired_size", "resident_size",
            "phys_footprint", "proc_start_abstime", "proc_exit_abstime", "child_user_time", "child_system_time",
            "child_pkg_idle_wkups", "child_interrupt_wkups", "child_pageins", "child_elapsed_abstime",
            "diskio_bytesread", "diskio_byteswritten")]

    libproc = ctypes.CDLL("/usr/lib/libproc.dylib")
    libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.POINTER(RusageInfoV2)]
    return libproc, RusageInfoV2


def memory_mb() -> dict:
    """Resident and private memory of this process (Windows: working set and private bytes; macOS: resident size
    and physical footprint, which includes Metal allocations), for leak checks."""
    if sys.platform == "darwin":
        import ctypes
        import os

        libproc, RusageInfoV2 = _darwin_memory_api()
        info = RusageInfoV2()
        if libproc.proc_pid_rusage(os.getpid(), 2, ctypes.byref(info)) != 0:  # 2 = RUSAGE_INFO_V2
            return {}
        return {"rss_mb": info.resident_size >> 20, "private_mb": info.phys_footprint >> 20}
    if sys.platform != "win32":
        return {}
    import ctypes

    kernel32, Counters = _process_memory_api()
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


def _actor_entry(*args):
    import signal

    # The parent handles terminal Ctrl-C and asks every actor to finish its rollout and flush labels.
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    _actor(*args)


def _actor(connection, buffer_names, weight_names, n_envs, steps, begin, end, pools, seed, path, model_config):
    sys.path.insert(0, path)
    import torch

    import numbrion as nb
    from training.dex import load_dex
    from training.encoder import Encoder
    from training.league import SEARCH, SELF, SNAPSHOT
    from training.model import Model
    from training.ppo import ko_shaping, sample
    from training.scripted import HeuristicAgent, forced_logprobs

    torch.set_num_threads(1)
    R = 2 * n_envs
    label_only = model_config.get("label_only", False)
    buffers = [SharedArrays({} if label_only else buffer_layout(steps, R, model_config.get("teacher", False)), names)
               for names in buffer_names]
    weights = SharedArrays({"learner": ((model_config["size"],), np.float32),
                            "opponent": ((model_config["size"],), np.float32)}, weight_names)
    dex = load_dex()
    architecture = dict(action_features=model_config["action_features"], token_features=model_config["token_features"],
                        joint_features=model_config["joint_features"])
    learner = Model(dex, model_config["width"], model_config["layers"], **architecture).eval()
    opponent = Model(dex, model_config["width"], model_config["layers"], **architecture).eval()
    guards = MoveGuards(**model_config["guards"])
    layout = weight_layout(learner)
    count = end - begin
    rows = slice(2 * begin, 2 * end)
    env = nb.BatchEnv(count, [str(p) for p in pools], seed=seed, threads=1, log=True)
    encoder = Encoder(2 * count)
    rng = np.random.default_rng(seed)
    if model_config.get("scripted", "heuristic") == "smart":
        from training.smart import SmartHeuristicAgent

        heuristic = SmartHeuristicAgent(rng, dex, temperature=model_config.get("scripted_temperature", 0.0))
    else:
        heuristic = HeuristicAgent(rng)
    teacher = None  # the smart player labelling the learner's decisions, made for the first teaching rollout
    searcher = None
    if model_config.get("search") is not None:
        from training.search_opponent import LabelWriter, OpponentSearch, SearchOpponent, load_bench_sets

        labels_dir = model_config.get("search_labels")
        value_model = None
        if model_config.get("search_value"):
            value_state = torch.load(model_config["search_value"], map_location="cpu", weights_only=False)
            value_model = Model.from_state(dex, value_state["model"], value_state.get("width", 128),
                                           value_state.get("layers", 3)).eval()
            del value_state
        searcher = SearchOpponent(str(dex.path), guards, OpponentSearch(**model_config["search"]), seed=seed + 7,
                                  labels=LabelWriter(labels_dir, tag=f"a{begin}-",
                                                     size=model_config.get("search_shard", 256)) if labels_dir else None,
                                  bench_sets=load_bench_sets(pools), value_model=value_model,
                                  fair_labels=model_config.get("search_fair_labels", False))
    search_both = bool(model_config.get("search_both", False))
    explore = float(model_config.get("search_explore", 0.0))
    explore_rng = np.random.default_rng([seed, 2])  # its own stream: exploring never moves the sampling stream
    versions = {"learner": -1, "opponent": -1}
    partner = np.arange(2 * count) ^ 1

    def observe(result):
        if hasattr(encoder, "encode_env"):
            encoder.encode_env(env)
            return encoder.buffers
        observations = [env.observe(e, s) for e in range(count) for s in (0, 1)]
        return encoder.encode_batch(observations)

    def tensors(b, index=slice(None)):
        return {k: torch.from_numpy(np.ascontiguousarray(getattr(b, k)[index])) for k in OBS_KEYS}

    result = env.reset()
    b = observe(result)
    potential = np.zeros(2 * count, dtype=np.float32)  # KO-shaping potential per row (training.ppo.ko_shaping)
    try:
        while True:
            command, argument = connection.recv()
            if command == "close":
                capped = sum(len(rows) for rows in searcher.pending.values()) if searcher is not None else 0
                if searcher is not None:
                    searcher.close()
                connection.send(("closed", dict(search_capped_at_close=capped,
                                                search_written=searcher.labels.written
                                                if searcher is not None and searcher.labels is not None else 0)))
                break
            # command == "collect"
            started = time.perf_counter()
            out = buffers[argument["buffer"]].arrays
            for key, model in (("learner", learner), ("opponent", opponent)):
                if argument[key + "_version"] != versions[key]:
                    model.load_state_dict(read_weights(weights.arrays[key], layout))
                    versions[key] = argument[key + "_version"]
            # ---- opponent assignment (training.league): side 2 of each env is the learner, a snapshot or scripted.
            kinds = np.asarray(argument["opponents"][begin:end], dtype=np.int8)
            opp_rows = np.zeros(2 * count, dtype=bool)  # side-2 rows not played (and not trained) by the learner
            opp_rows[1::2] = kinds != SELF
            snapshot_rows = np.zeros(2 * count, dtype=bool)
            snapshot_rows[1::2] = kinds == SNAPSHOT
            search_rows = np.zeros(2 * count, dtype=bool)
            search_rows[1::2] = kinds == SEARCH
            if search_both:
                search_rows[0::2] = kinds == SEARCH
            if search_rows.any() and searcher is None:
                raise RuntimeError("SEARCH opponents need Actors(search=...)")
            opp_index = np.flatnonzero(snapshot_rows)  # rows the opponent snapshot plays
            scripted_index = np.flatnonzero(opp_rows & ~snapshot_rows & ~search_rows)  # rows the heuristic plays
            policy_rows = ~opp_rows | snapshot_rows  # learner and pool snapshot rows
            # Per-env finished games, side-1 score (win 1, tie 0.5) and battle turns; "clean" games began during
            # this rollout, so they were played entirely against this rollout's opponent.
            results = {k: np.zeros(count) for k in RESULT_KEYS}
            began_here = np.zeros(count, dtype=bool)
            # ---- end opponent assignment
            ko_bonus = argument["ko_bonus"]
            teach = bool(argument.get("teacher")) and "teacher_a0" in out
            if teach and teacher is None:
                from training.smart import SmartHeuristicAgent

                # Its own instance (first/second keep per-call context) and random stream, so labelling never moves
                # the actor's sampling stream or the scripted opponent's.
                teacher = SmartHeuristicAgent(np.random.default_rng([seed, 1]), dex,
                                              temperature=model_config.get("teacher_temperature", 0.0))
            stats = dict(completed=0, turns=0, guarded=0, crashes=[])
            inference = teaching = searching = 0.0
            explored_count = 0
            if searcher is not None:
                searcher.stats.update(searched=0, forced=0, cells=0, seconds=0.0, rejected_labels=0)
                labels_before = searcher.released
            for t in range(steps):
                if not label_only:
                    for k in OBS_KEYS:
                        out[k][t, rows] = getattr(b, k)
                active = result["needs_action"].reshape(-1).copy()
                train_rows = active & ~opp_rows
                mask0 = result["mask0"].reshape(-1, N_ACTIONS).copy()
                mask0[~active] = False
                mask0[~active, PASS] = True
                # Explored search rows play the learner's own sample, so they get the policy's guarded masks too.
                explored = search_rows & active & (explore_rng.random(2 * count) < explore) if explore > 0 else None
                guard_rows = policy_rows if explored is None else policy_rows | explored
                mask0[guard_rows], guarded = guards.apply(mask0[guard_rows], b.guard[guard_rows], 0)
                stats["guarded"] += int((guarded & train_rows[guard_rows]).sum())
                t0 = time.perf_counter()
                with torch.no_grad():
                    x = tensors(b)
                    state = learner.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
                    if not label_only:
                        value = learner.value(state, x["ids"][partner], x["floats"][partner, 0:6]).numpy()
                    lp0_t, vectors0 = learner.slot0(state, torch.from_numpy(mask0))
                    lp0 = lp0_t.numpy().copy()
                    if len(opp_index):
                        y = tensors(b, opp_index)
                        opp_state = opponent.trunk(y["ids"], y["floats"], y["field"], y["action_features"], y["token_features"])
                        opp_lp0, opp_vectors0 = opponent.slot0(opp_state, torch.from_numpy(mask0[opp_index]))
                        lp0[opp_index] = opp_lp0.numpy()
                    if len(scripted_index):  # opponent selection: the heuristic's codes as one-hot log-probs
                        lp0[scripted_index] = forced_logprobs(heuristic.first(
                            b.arrays(), scripted_index, mask0[scripted_index]))
                    searched_index = np.flatnonzero(search_rows & active)
                    if len(searched_index):  # the search's pairs as one-hot log-probs
                        t1 = time.perf_counter()
                        chosen = {}  # row -> pair
                        for side in (0, 1):
                            side_rows = searched_index[searched_index % 2 == side]
                            if len(side_rows):
                                picks = searcher.act(learner, env, encoder, state, {**b.arrays(), "guard": b.guard},
                                                     side_rows // 2, result["needs_action"], argument["minutes"],
                                                     argument.get("label_teacher", "selfplay-search"), side=side)
                                chosen.update({2 * e + side: pair for e, pair in picks.items()})
                        if explored is not None:  # explored rows keep the learner's sample (label: the search's)
                            explored_count += int(explored[searched_index].sum())
                            searched_index = searched_index[~explored[searched_index]]
                        pairs = np.array([chosen[int(r)] for r in searched_index], dtype=np.int64).reshape(-1, 2)
                        lp0[searched_index] = forced_logprobs(pairs[:, 0])
                        searching += time.perf_counter() - t1
                    a0 = sample(lp0, rng)
                    mask1 = env.mask_slot1(np.where(active, a0, -1).reshape(count, 2).astype(np.int32)).reshape(-1, N_ACTIONS).copy()
                    mask1[~active] = False
                    mask1[~active, PASS] = True
                    mask1[guard_rows], guarded = guards.apply(mask1[guard_rows], b.guard[guard_rows], 1)
                    stats["guarded"] += int((guarded & train_rows[guard_rows]).sum())
                    lp1 = learner.slot1(state, vectors0, torch.from_numpy(a0.astype(np.int64)), torch.from_numpy(mask1)).numpy().copy()
                    if len(opp_index):
                        lp1[opp_index] = opponent.slot1(opp_state, opp_vectors0, torch.from_numpy(a0[opp_index].astype(np.int64)),
                                                        torch.from_numpy(mask1[opp_index])).numpy()
                    if len(scripted_index):
                        lp1[scripted_index] = forced_logprobs(heuristic.second(scripted_index, mask1[scripted_index]))
                    if len(searched_index):
                        lp1[searched_index] = forced_logprobs(pairs[:, 1])
                    a1 = sample(lp1, rng)
                inference += time.perf_counter() - t0
                if "teacher_a0" in out:  # -1: no label
                    labels = np.full((2, 2 * count), -1, dtype=np.int64)
                    index = np.flatnonzero(train_rows)
                    if teach and len(index):
                        t2 = time.perf_counter()
                        # Guarded masks: the labels are legal where the policy's log-probs are. Slot b answers the
                        # learner's own slot-a code, so its label needs no extra forward pass in the PPO loss.
                        labels[0, index] = teacher.first(b.arrays(), index, mask0[index])
                        labels[1, index] = teacher.second_given(index, a0[index], mask1[index])
                        teaching += time.perf_counter() - t2
                    out["teacher_a0"][t, rows], out["teacher_a1"][t, rows] = labels
                actions = np.stack((a0, a1), -1).reshape(count, 2, 2).astype(np.int32)
                actions[~result["needs_action"]] = -1
                if not label_only:
                    out["mask0"][t, rows], out["mask1"][t, rows] = mask0, mask1
                    out["a0"][t, rows], out["a1"][t, rows] = a0, a1
                    out["logp"][t, rows] = lp0[np.arange(len(a0)), a0] + lp1[np.arange(len(a1)), a1]
                    out["value"][t, rows] = value
                    out["train"][t, rows] = train_rows
                result = env.step(actions)
                if result["illegal"].any():
                    raise RuntimeError("A sampled action violated the exact mask")
                if result["aborted"].any():  # an engine panic aborted a battle, which was replaced; keep the report
                    stats["crashes"].extend(env.crashes())
                if searcher is not None and searcher.labels is not None:  # outcomes of the pending search labels
                    searcher.end_step(result["done"], result["winner"], result["aborted"])
                b = observe(result)
                done = result["done"]
                reward = result["reward"].reshape(-1).astype(np.float32)
                # Potential-based, sums to zero over a battle (training.ppo.ko_shaping); the stored potential
                # carries across rollouts, so a decayed or zero bonus still settles what was credited.
                shaped, potential = ko_shaping(potential, b.fainted, np.repeat(done, 2), ko_bonus)
                reward = reward + shaped
                if not label_only:
                    out["reward"][t, rows] = reward
                    out["done"][t, rows] = np.repeat(done, 2)
                stats["completed"] += int(done.sum())
                stats["turns"] += int(result["final_turns"][done].sum())
                # ---- opponent results (training.league)
                score = np.where(result["winner"] < 0, 0.5, (result["winner"] == 0).astype(float))
                clean = done & began_here
                results["games"] += done
                results["score"] += np.where(done, score, 0.0)
                results["clean_games"] += clean
                results["clean_score"] += np.where(clean, score, 0.0)
                results["battle_turns"] += np.where(done, result["final_turns"], 0)
                began_here |= done
            if not label_only:
                with torch.no_grad():
                    x = tensors(b)
                    state = learner.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
                    out["bootstrap"][rows] = learner.value(state, x["ids"][partner], x["floats"][partner, 0:6]).numpy()
            stats["seconds"] = time.perf_counter() - started
            stats["inference_seconds"] = inference
            stats["teacher_seconds"] = teaching
            stats["search_seconds"] = searching
            if searcher is not None:
                stats["search_decisions"] = searcher.stats["searched"]
                stats["search_forced"] = searcher.stats["forced"]
                stats["search_cells"] = searcher.stats["cells"]
                stats["search_unswapped"] = searcher.stats["unswapped"]
                stats["search_labels"] = searcher.released - labels_before  # rows whose battle ended
                stats["search_pending"] = sum(len(rows) for rows in searcher.pending.values())
                stats["search_written"] = searcher.labels.written if searcher.labels else 0
                stats["search_explored"] = explored_count
                stats["search_rejected_labels"] = searcher.stats["rejected_labels"]
            stats.update({k: v.tolist() for k, v in results.items()})  # per env, this actor's slice
            stats.update(memory_mb())
            connection.send(stats)
    except Exception:
        import traceback
        connection.send(("error", traceback.format_exc()))
        raise
    finally:
        try:
            if searcher is not None:
                searcher.close()  # also preserve buffered / pending labels after an actor exception
        finally:
            for buffer in buffers:
                buffer.close()
            weights.close()


class Actors:
    """``n_envs`` battles over ``workers`` actor processes, two rollout buffers of ``steps`` boundaries."""

    def __init__(self, n_envs: int, steps: int, pools: list[Path], seed: int, workers: int, model,
                 scripted: str = "heuristic", scripted_temperature: float = 0.0, guards: MoveGuards = MoveGuards(),
                 teacher: bool = False, teacher_temperature: float = 0.0, search: dict | None = None,
                 search_labels: Path | None = None, search_value: Path | None = None, search_shard: int = 256,
                 search_both: bool = False, search_explore: float = 0.0, search_fair_labels: bool = False,
                 label_only: bool = False):
        """``teacher``: the buffers hold teacher labels, written in rollouts started with ``teacher=True`` (smart
        player with Gumbel noise ``teacher_temperature``). ``search``: training.search_opponent.OpponentSearch
        fields for SEARCH envs (None: no search opponent); ``search_labels``: directory for its label shards
        (``search_shard`` rows each); ``search_value``: checkpoint for the search's leaf values (None: the learner);
        ``search_both`` / ``search_explore``: see the module docstring (defaults: side 2 only, no exploration).
        ``search_fair_labels`` excludes decisions with nonpublic masks or an incompletely replaced unseen bench
        from label output, without changing play. shallow_labels.py enables it. ``label_only`` collects search
        labels and stats without allocating PPO rollout buffers or evaluating unused rollout values."""
        if label_only and teacher:
            raise ValueError("label_only does not collect PPO teacher labels")
        self.n_envs, self.steps, self.teacher = n_envs, steps, teacher
        self.buffers = [SharedArrays({} if label_only else buffer_layout(steps, 2 * n_envs, teacher)) for _ in range(2)]
        self.layout = weight_layout(model)
        size = sum(int(np.prod(shape)) for _, shape, _ in self.layout)
        self.weights = SharedArrays({"learner": ((size,), np.float32), "opponent": ((size,), np.float32)})
        self.versions = {"learner": 0, "opponent": 0}
        config = dict(size=size, width=model.width, layers=len(model.blocks), joint_features=model.joint_features,
                      action_features=model.n_action_features, token_features=model.n_token_features,
                      scripted=scripted, scripted_temperature=scripted_temperature, guards=asdict(guards),
                      teacher=teacher, teacher_temperature=teacher_temperature, search=search,
                      search_labels=None if search_labels is None else str(search_labels),
                      search_value=None if search_value is None else str(search_value), search_shard=search_shard,
                      search_both=search_both, search_explore=search_explore, search_fair_labels=search_fair_labels,
                      label_only=label_only)
        context = mp.get_context("spawn")
        bounds = np.linspace(0, n_envs, workers + 1).astype(int)
        path = str(Path(__file__).resolve().parents[1])
        self.connections, self.processes = [], []
        for w in range(workers):
            parent, child = context.Pipe()
            process = context.Process(target=_actor_entry, daemon=True, args=(
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

    def start(self, buffer: int, opponents: np.ndarray, ko_bonus: float, teacher: bool = False,
              minutes: float = 0.0, label_teacher: str = "selfplay-search") -> None:
        """``opponents``: per-env opponent kind (training.league SELF / SNAPSHOT / HEURISTIC / SEARCH) for this
        rollout. ``teacher``: label the learner's decisions (otherwise the labels are all -1). ``minutes``: training
        minutes of the learner weights (the search labels' ``teacher_minutes``); ``label_teacher``: their ``teacher``."""
        if teacher and not self.teacher:
            raise ValueError("teacher labels need Actors(teacher=True)")
        argument = dict(buffer=buffer, opponents=np.asarray(opponents).tolist(), ko_bonus=ko_bonus,
                        learner_version=self.versions["learner"], opponent_version=self.versions["opponent"],
                        teacher=bool(teacher), minutes=float(minutes), label_teacher=str(label_teacher))
        for connection in self.connections:
            connection.send(("collect", argument))
        self.pending = True

    def wait(self) -> dict:
        replies = [connection.recv() for connection in self.connections]
        self.pending = False
        for reply in replies:
            if isinstance(reply, tuple) and reply and reply[0] == "error":
                raise RuntimeError("actor failed:\n" + reply[1])
        totals = {k: sum(r[k] for r in replies) for k in ("completed", "turns", "guarded")}
        totals["crashes"] = [c for r in replies for c in r["crashes"]]
        totals["seconds"] = max(r["seconds"] for r in replies)
        totals["inference_seconds"] = max(r["inference_seconds"] for r in replies)
        totals["teacher_seconds"] = max(r["teacher_seconds"] for r in replies)
        totals["search_seconds"] = max(r["search_seconds"] for r in replies)
        for key in ("search_decisions", "search_forced", "search_cells", "search_labels", "search_unswapped",
                    "search_pending", "search_written", "search_explored", "search_rejected_labels"):
            totals[key] = sum(r.get(key, 0) for r in replies)
        # Per-env opponent results (actors own consecutive env slices, in order).
        for key in RESULT_KEYS:
            totals[key] = np.concatenate([np.asarray(r[key], dtype=np.float64) for r in replies])
        for key in ("rss_mb", "private_mb"):  # the largest actor
            if all(key in r for r in replies):
                totals["actor_" + key] = max(r[key] for r in replies)
        return totals

    def close(self) -> dict:
        # Queue every close first. A pending rollout's stats can precede the close acknowledgement.
        totals = dict(search_capped_at_close=0, search_written=0)
        for connection in self.connections:
            try:
                connection.send(("close", None))
            except (BrokenPipeError, EOFError, OSError):
                pass
        for connection in self.connections:
            try:
                while True:
                    reply = connection.recv()
                    if isinstance(reply, tuple) and reply[0] == "closed":
                        for key in totals:
                            totals[key] += reply[1][key]
                        break
            except (BrokenPipeError, EOFError, OSError):
                pass
            finally:
                connection.close()
        self.pending = False
        for process in self.processes:
            process.join(timeout=5)
        for buffer in self.buffers:
            buffer.close(unlink=True)
        self.weights.close(unlink=True)
        return totals

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
