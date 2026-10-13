"""A shallow search opponent inside the training actors (train_selfplay.py ``--search-fraction``).

Side 2 of a ``SEARCH`` env (training.league) is played by one-turn search (training.search) with the learner's own
current weights. Unlike the protocol-driven SearchPlayer it searches the env's true battle: one world, the actual
state (no world building, no log replay), the learner's real observation row for the opponent's candidates, and both
sides' trackers from the actor's encoder. All search envs of an actor at a boundary share one grid call, one batched
value-head call and one batched matrix-game solve, so the cost is a few small CPU batches per step.

Defaults (``ours`` 4, ``theirs`` 3, ``seeds`` 1) are the 11-cell setting that kept 71% vs the m535 policy with
sampled worlds (full search 76.7%; scratch/shallow-search, 2026-10-13).

Labels: every searched decision can become a distillation row (search_labels.py's format), written as
``labels-<run>-<seq>.npz`` shards into a ``--distill-stream`` directory. These break the fair-information rule for
training targets (docs/design/SEARCH-PREREQUISITES.md) on purpose, for speed: world building would cost ~25-35 ms per
decision instead of ~4. The true battle is a sample from the searcher's posterior over hidden information, so in
expectation the targets equal one-world search with an exact world sampler; like any one-world search they average
per-world best replies (strategy fusion) where multi-world search shares one strategy across worlds. Judge them by
head-to-head results of the trained policy, never by the searcher's own score.
"""
from __future__ import annotations

import os
import time
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

import numpy as np
import torch

from .guards import MoveGuards
from .search import _take, pair_candidates, solve_batch

N_ACTIONS = 47
LABEL_KEYS = ("ids", "floats", "field", "action_features", "token_features")
LABEL_PAIRS = 8  # candidate columns per label row (search_labels.py's width; the trainer keeps the top --distill-pairs)
SEARCH_SIDE = 1


@dataclass
class OpponentSearch:
    ours: int = 4
    theirs: int = 3
    seeds: int = 1
    first_codes: int = 8            # slot-a codes expanded when enumerating pairs
    prior_temperature: float = 0.5  # ordering of our candidates (the base policy's temperature, as SearchPlayer)
    anchor: float = 0.05
    opponent_anchor: float = 0.1
    iterations: int = 200


class LabelWriter:
    """Search labels as atomic ``labels-<run>-<seq>.npz`` shards (``.npz.tmp`` then rename), ``size`` rows each.
    Every 20th shard is a holdout shard, like search_labels.py's ShardWriter."""

    def __init__(self, out: Path, size: int = 256, tag: str = ""):
        self.out, self.size = Path(out), size
        self.out.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-%f")
        self.run = f"{stamp}-{tag}{uuid.uuid4().hex[:12]}"
        self.rows: list[dict] = []
        self.minutes: list[float] = []
        self.sequence = 0
        self.written = 0

    def add(self, row: dict, minutes: float) -> None:
        self.rows.append(row)
        self.minutes.append(minutes)
        if len(self.rows) >= self.size:
            self.flush()

    def flush(self) -> None:
        if not self.rows:
            return
        arrays = {key: np.stack([row[key] for row in self.rows]) for key in self.rows[0]}
        # The oldest weights in the shard, so age-based retirement never keeps a row too long.
        arrays.update(teacher=np.asarray("selfplay-search"), teacher_minutes=np.asarray(min(self.minutes), np.float64),
                      created=np.asarray(time.time(), np.float64),
                      holdout=np.asarray(self.sequence % 20 == 19, dtype=bool))
        path = self.out / f"labels-{self.run}-{self.sequence:06d}.npz"
        temporary = path.with_suffix(".npz.tmp")
        with temporary.open("wb") as stream:
            np.savez_compressed(stream, **arrays)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        self.written += len(self.rows)
        self.sequence += 1
        self.rows, self.minutes = [], []


class SearchOpponent:
    """Search for the side-2 rows of the actor's SEARCH envs (see the module docstring)."""

    def __init__(self, dex_path: str, guards: MoveGuards, config: OpponentSearch | None = None, seed: int = 0,
                 labels: LabelWriter | None = None, threads: int = 1):
        import numbrion

        self.cfg = config or OpponentSearch()
        self.guards = guards
        self.grid = numbrion.SearchGrid(dex_path, threads)
        self.rng = np.random.default_rng(seed)
        self.labels = labels
        self.stats = dict(searched=0, forced=0, cells=0, seconds=0.0)

    @torch.no_grad()
    def act(self, model, env, encoder, state: dict, arrays: dict, envs: np.ndarray, needs_action: np.ndarray,
            minutes: float) -> dict:
        """Choices for side 2 of ``envs`` (local env indices whose side 2 must act).

        ``state``: the learner trunk over all of the actor's rows (row 2e + side); ``arrays``: the encoder buffers
        (OBS_KEYS and ``guard``) of the same rows; ``needs_action`` [envs, 2]. Returns {env: (a0, a1)}."""
        started = time.perf_counter()
        cfg = self.cfg
        battles = [env.clone_battle(int(e)) for e in envs]
        own = [b.legal_mask(SEARCH_SIDE) for b in battles]
        choices, searched = {}, []
        for i, e in enumerate(envs):
            legal = np.argwhere(own[i][1])
            if len(legal) == 1:  # forced: no search, no label
                choices[int(e)] = tuple(map(int, legal[0]))
                self.stats["forced"] += 1
            elif len(legal):
                searched.append(i)
        if not searched:
            self.stats["seconds"] += time.perf_counter() - started
            return choices
        envs_s = np.asarray([envs[i] for i in searched])
        battles = [battles[i] for i in searched]
        own = [own[i] for i in searched]
        rows = 2 * envs_s + SEARCH_SIDE
        ours = pair_candidates(model, _take(state, torch.from_numpy(rows)), arrays["guard"][rows], own, cfg.ours,
                               cfg.first_codes, cfg.prior_temperature, self.guards)
        acting = [i for i, e in enumerate(envs_s) if needs_action[e, 1 - SEARCH_SIDE]]
        theirs: list = [None] * len(envs_s)
        if acting:
            their_rows = 2 * envs_s[acting] + (1 - SEARCH_SIDE)
            masks = [battles[i].legal_mask(1 - SEARCH_SIDE) for i in acting]
            found = pair_candidates(model, _take(state, torch.from_numpy(their_rows)), arrays["guard"][their_rows],
                                    masks, cfg.theirs, cfg.first_codes, 1.0, self.guards)
            for i, (pairs, logp, _) in zip(acting, found):
                if len(pairs):
                    theirs[i] = (pairs, logp)
        # Cells: (env, our pair, their pair, seed); one seed per (env, seed index), shared by all pairs.
        B = len(envs_s)
        N = max(len(p) for p, _, _ in ours)
        M = max(len(t[0]) if t is not None else 1 for t in theirs)
        cell_world, cell_actions, cell_index, seeds = [], [], [], []
        for k in range(B):
            our_pairs = ours[k][0]
            their_pairs = theirs[k][0] if theirs[k] is not None else np.full((1, 2), -1, dtype=np.int32)
            for s in range(cfg.seeds):
                seed = self.rng.integers(0, 1 << 16, size=4)
                for i, pair in enumerate(our_pairs):
                    for j, their_pair in enumerate(their_pairs):
                        act = np.full((2, 2), -1, dtype=np.int32)
                        act[SEARCH_SIDE] = pair
                        act[1 - SEARCH_SIDE] = their_pair
                        cell_world.append(k)
                        cell_actions.append(act)
                        cell_index.append((k, i, j))
                        seeds.append(seed)
        trackers = [(encoder.snapshot(int(2 * e)), encoder.snapshot(int(2 * e + 1))) for e in envs_s]
        out = self.grid.run(battles, np.stack(cell_actions), trackers, SEARCH_SIDE,
                            world=np.asarray(cell_world, dtype=np.int64), seeds=np.stack(seeds).astype(np.int64),
                            allow_live=True)  # the true battle on purpose (module docstring)
        n = len(cell_actions)
        values = np.zeros(n, dtype=np.float32)
        live = np.flatnonzero(out["eval_side"] >= 0)
        if len(live):
            leaf = model.trunk(*(torch.from_numpy(out[key][live]) for key in ("ids", "floats", "field")),
                               torch.zeros(len(live), 2, 40, 0), torch.from_numpy(out["token_features"][live]))
            v = model.value(leaf, torch.from_numpy(out["partner_ids"][live]),
                            torch.from_numpy(out["partner_floats"][live])).clamp(-1, 1).numpy()
            values[live] = np.where(out["eval_side"][live] == SEARCH_SIDE, v, -v)
        done = out["ended"]
        winner = out["winner"]
        values[done] = np.where(winner[done] == SEARCH_SIDE, 1.0, np.where(winner[done] < 0, 0.0, -1.0))
        payoffs = np.zeros((B, N, M))
        counts = np.zeros((B, N, M))
        cell = np.asarray(cell_index)
        np.add.at(payoffs, (cell[:, 0], cell[:, 1], cell[:, 2]), values)
        np.add.at(counts, (cell[:, 0], cell[:, 1], cell[:, 2]), 1)
        payoffs /= np.maximum(counts, 1)
        our_prior = np.zeros((B, N))
        their_prior = np.zeros((B, M))
        for k in range(B):
            pairs, _, full = ours[k]
            logp = np.array([full[(int(a), int(b))][0] for a, b in pairs])
            our_prior[k, :len(pairs)] = np.exp(logp - logp.max())
            if theirs[k] is None:
                their_prior[k, 0] = 1.0
            else:
                logp = theirs[k][1].astype(np.float64)  # tempered at 1.0: the log-prior itself
                their_prior[k, :len(logp)] = np.exp(logp - logp.max())
        x, q = solve_batch(payoffs, our_prior, their_prior, cfg.anchor, cfg.opponent_anchor, cfg.iterations)
        for k, e in enumerate(envs_s):
            pairs = ours[k][0]
            best = int(np.argmax(x[k, :len(pairs)]))
            choices[int(e)] = (int(pairs[best][0]), int(pairs[best][1]))
            if self.labels is not None:
                self.labels.add(self._label(arrays, int(rows[k]), own[k], ours[k], x[k], q[k]), minutes)
        self.stats["searched"] += B
        self.stats["cells"] += n
        self.stats["seconds"] += time.perf_counter() - started
        return choices

    def _label(self, arrays: dict, row: int, mask: tuple, candidates: tuple, x: np.ndarray, q: np.ndarray) -> dict:
        """One search_labels.make_label row for the searched decision at encoder row ``row``."""
        pairs, _, full = candidates
        n = len(pairs)
        guard = arrays["guard"][row:row + 1]
        mask0, _ = self.guards.apply(mask[0][None], guard, 0)
        mask1 = np.zeros((LABEL_PAIRS, N_ACTIONS), dtype=bool)
        mask1[:n], _ = self.guards.apply(mask[1][pairs[:, 0]], np.repeat(guard, n, 0), 1)
        padded = np.full((LABEL_PAIRS, 2), -1, dtype=np.int16)
        padded[:n] = pairs
        weights = np.zeros(LABEL_PAIRS, np.float32)
        values = np.zeros(LABEL_PAIRS, np.float32)
        prior = np.full(LABEL_PAIRS, -30.0, np.float32)
        w = x[:n] / x[:n].sum()
        weights[:n], values[:n] = w, q[:n]
        prior[:n] = [full[(int(a), int(b))][0] for a, b in pairs]
        row_arrays = {key: arrays[key][row].copy() for key in LABEL_KEYS}
        row_arrays.update(mask0=mask0[0], mask1=mask1, pairs=padded, x=weights, q=values, prior=prior,
                          value=np.float32(float(w @ q[:n])))
        return row_arrays
