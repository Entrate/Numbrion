"""A shallow search opponent inside the training actors (train_selfplay.py ``--search-fraction``).

Side 2 of a ``SEARCH`` env (training.league) is played by one-turn search (training.search) with the learner's own
current weights. Unlike the protocol-driven SearchPlayer it searches a copy of the env's battle: one world, the
actual state (no world building, no log replay), and both sides' trackers from the actor's encoder. The learner's
Pokemon the searcher has never seen are first replaced by sets drawn from the training team pool (``bench_sets``;
``Battle.replace_hidden_set``, in place, no replay), and the learner's candidates come from its observation of that
copy, so the search does not know the learner's bench. What it still knows: the unrevealed moves, item, ability,
Tera type and stats of the learner's Pokemon it has seen. All search envs of an actor at a boundary share one grid call, one batched
value-head call and one batched matrix-game solve, so the cost is a few small CPU batches per step.

``act(..., side=0)`` / ``hide_bench(battle, 0)`` search side 1 the same way against side 2 (shallow_labels.py searches
both sides of every env; the default, side 2, is train_selfplay's search opponent).

Defaults (``ours`` 4, ``theirs`` 3, ``seeds`` 1) are the 11-cell setting that kept 71% vs the m535 policy with
sampled worlds (full search 76.7%; scratch/shallow-search, 2026-10-13).

Labels: every searched decision can become a distillation row (search_labels.py's format, value fields included),
written as ``labels-<run>-<seq>.npz`` shards into a ``--distill-stream`` directory (or by shallow_labels.py). A row
waits in ``pending`` until its battle ends, which sets ``outcome`` (+1 / -1 / 0 from the searching side) and
``remaining_steps`` (env boundaries until the terminal transition, every boundary of the env counted as GAE counts
them: forced switches and waits included). Rows of a battle an engine panic aborted, and of battles still running at
``close``, are written as capped (NaN / -1): policy targets without a terminal value target. ``partner_ids`` /
``partner_floats`` are the other row of the same env (the opponent's own view, train_selfplay.partner_view) and
``foe_match`` the searching row's own. ``value_model``: a fixed network (any size) for the leaf values instead of the
policy's own value head; the candidates and priors still come from the policy. These still break the fair-information rule for
training targets (docs/design/SEARCH-PREREQUISITES.md) for the seen Pokemon, on purpose, for speed: world building would
cost ~25-35 ms per decision instead of ~4-5. Like any one-world search the targets average per-world best replies
(strategy fusion) where multi-world search shares one strategy across worlds. Judge them by head-to-head results of
the trained policy, never by the searcher's own score.
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
    Every 20th shard is a holdout shard, like search_labels.py's ShardWriter. Rows are grouped by ``teacher`` (the
    shard's ``teacher`` metadata), so a shard never mixes teachers; ``teacher_minutes`` is the group's oldest."""

    def __init__(self, out: Path, size: int = 256, tag: str = ""):
        self.out, self.size = Path(out), size
        self.out.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-%f")
        self.run = f"{stamp}-{tag}{uuid.uuid4().hex[:12]}"
        self.groups: dict[str, tuple[list, list]] = {}  # teacher -> (rows, minutes)
        self.sequence = 0
        self.written = 0

    @property
    def buffered(self) -> int:
        return sum(len(rows) for rows, _ in self.groups.values())

    def add(self, row: dict, minutes: float, teacher: str = "selfplay-search") -> None:
        rows, times = self.groups.setdefault(teacher, ([], []))
        rows.append(row)
        times.append(minutes)
        if len(rows) >= self.size:
            self._write(teacher)

    def flush(self) -> None:
        for teacher in list(self.groups):
            self._write(teacher)

    def _write(self, teacher: str) -> None:
        rows, minutes = self.groups.pop(teacher, ([], []))
        if not rows:
            return
        arrays = {key: np.stack([row[key] for row in rows]) for key in rows[0]}
        # The oldest weights in the shard, so age-based retirement never keeps a row too long.
        arrays.update(teacher=np.asarray(teacher), teacher_minutes=np.asarray(min(minutes), np.float64),
                      created=np.asarray(time.time(), np.float64),
                      holdout=np.asarray(self.sequence % 20 == 19, dtype=bool))
        path = self.out / f"labels-{self.run}-{self.sequence:06d}.npz"
        temporary = path.with_suffix(".npz.tmp")
        with temporary.open("wb") as stream:
            np.savez_compressed(stream, **arrays)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        self.written += len(rows)
        self.sequence += 1


def _base(species: str) -> str:
    """Species-clause key: the lowercase alphanumeric name before the first hyphen (forms share a key)."""
    return "".join(ch for ch in species.split("-")[0].lower() if ch.isalnum())


def load_bench_sets(paths) -> tuple[list[str], list[str]]:
    """Every single packed set of the packed-team files ``paths`` and its species-clause key."""
    sets, keys = [], []
    for path in paths:
        for line in Path(path).read_text().splitlines():
            for packed in line.strip().split("]"):
                fields = packed.split("|")
                if len(fields) > 2:
                    sets.append(packed)
                    keys.append(_base(fields[0] or fields[1]))  # the name: the display species (ids drop hyphens)
    if not sets:
        raise ValueError(f"no packed sets in {paths}")
    return sets, keys


class SearchOpponent:
    """Search for the side-2 rows of the actor's SEARCH envs (see the module docstring)."""

    def __init__(self, dex_path: str, guards: MoveGuards, config: OpponentSearch | None = None, seed: int = 0,
                 labels: LabelWriter | None = None, threads: int = 1, bench_sets: tuple | None = None,
                 value_model=None):
        """``bench_sets``: (packed sets, species keys) from ``load_bench_sets`` to hide the learner's unseen
        Pokemon; None searches the true battle (the learner's bench included). ``value_model``: a fixed Model (any
        width / depth) for the leaf values; None uses the policy's own value head."""
        import numbrion

        self.cfg = config or OpponentSearch()
        self.guards = guards
        self.grid = numbrion.SearchGrid(dex_path, threads)
        self.rng = np.random.default_rng(seed)
        self.labels = labels
        self.bench_sets = bench_sets
        self.value_model = value_model
        self.pending: dict[int, list] = {}  # env -> [(row, minutes, teacher, step)] until its battle ends
        self.steps: np.ndarray | None = None  # boundaries each env's current battle has passed (see end_step)
        self.released = 0  # rows handed to ``labels`` (finished battles and capped rows)
        self.stats = dict(searched=0, forced=0, cells=0, seconds=0.0, swapped=0, unswapped=0)

    def end_step(self, done: np.ndarray, winner: np.ndarray, aborted: np.ndarray | None = None) -> None:
        """Call after every ``env.step`` (every env, searched or not): counts the boundary and releases the pending
        rows of the battles that ended with their ``outcome`` and ``remaining_steps``. An aborted battle (engine
        panic, reported as a winnerless end) has no outcome: its rows are released as capped."""
        if self.steps is None:
            self.steps = np.zeros(len(done), dtype=np.int64)
        self.steps += 1
        for e in np.flatnonzero(done):
            e = int(e)
            capped = aborted is not None and bool(aborted[e])
            self._release(e, None if capped else int(winner[e]), None if capped else int(self.steps[e]))
            self.steps[e] = 0

    def close(self) -> None:
        """Release the rows of battles still running as capped (NaN / -1) and write every buffered row."""
        for e in list(self.pending):
            self._release(e, None, None)
        if self.labels is not None:
            self.labels.flush()

    def _release(self, env: int, winner: int | None, steps: int | None) -> None:
        """``winner`` None (capped): NaN outcome and -1 steps; otherwise each row's outcome from its own side."""
        for row, minutes, teacher, step, side in self.pending.pop(env, []):
            row["outcome"] = np.float32(np.nan if winner is None else 0.0 if winner < 0 else
                                        1.0 if winner == side else -1.0)
            row["remaining_steps"] = np.int32(-1 if steps is None else steps - step - 1)
            self.labels.add(row, minutes, teacher)
            self.released += 1

    def hide_bench(self, battle, side: int = SEARCH_SIDE) -> None:
        """Replace the opponent's Pokemon the searching ``side`` has never seen with pool sets (distinct
        species-clause keys, none shared with the opponent's seen Pokemon). A set the engine refuses is retried;
        after 8 refusals the true set stays (counted in ``unswapped``)."""
        hidden = battle.hidden_team_indices(side)
        if not hidden or self.bench_sets is None:
            return
        sets, keys = self.bench_sets
        team = battle.teams[1 - side].split("]")
        used = {_base(team[i].split("|")[0] or team[i].split("|")[1]) for i in range(len(team)) if i not in hidden}
        for index in hidden:
            for _ in range(8):
                pick = int(self.rng.integers(len(sets)))
                if keys[pick] in used:
                    continue
                try:
                    battle.replace_hidden_set(side, int(index), sets[pick])
                except ValueError:
                    continue
                used.add(keys[pick])
                self.stats["swapped"] += 1
                break
            else:
                self.stats["unswapped"] += 1

    @torch.no_grad()
    def act(self, model, env, encoder, state: dict, arrays: dict, envs: np.ndarray, needs_action: np.ndarray,
            minutes: float, teacher: str = "selfplay-search", side: int = SEARCH_SIDE) -> dict:
        """Choices for ``side`` (default side 2) of ``envs`` (local env indices whose ``side`` must act).

        ``state``: the learner trunk over all of the actor's rows (row 2e + side); ``arrays``: the encoder buffers
        (OBS_KEYS and ``guard``) of the same rows; ``needs_action`` [envs, 2]. ``minutes`` / ``teacher``: the label
        shards' teacher metadata. Returns {env: (a0, a1)}."""
        started = time.perf_counter()
        cfg = self.cfg
        battles = [env.clone_battle(int(e)) for e in envs]
        own = [b.legal_mask(side) for b in battles]
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
        for battle in battles:
            self.hide_bench(battle, side)
        rows = 2 * envs_s + side
        ours = pair_candidates(model, _take(state, torch.from_numpy(rows)), arrays["guard"][rows], own, cfg.ours,
                               cfg.first_codes, cfg.prior_temperature, self.guards)
        acting = [i for i, e in enumerate(envs_s) if needs_action[e, 1 - side]]
        theirs: list = [None] * len(envs_s)
        if acting:
            their_rows = 2 * envs_s[acting] + (1 - side)
            masks = [battles[i].legal_mask(1 - side) for i in acting]
            if self.bench_sets is None:  # the learner's real observation row
                their_state, their_guard = _take(state, torch.from_numpy(their_rows)), arrays["guard"][their_rows]
            else:  # the learner's observation of the copy: its tracker with the copy's request (hidden bench)
                view = self.grid.encode_states([encoder.snapshot(int(r)) for r in their_rows],
                                               [battles[i].request_json(1 - side) for i in acting])
                their_state = model.trunk(*(torch.from_numpy(view[k]) for k in
                                            ("ids", "floats", "field", "action_features", "token_features")))
                their_guard = view["guard"]
            found = pair_candidates(model, their_state, their_guard, masks, cfg.theirs, cfg.first_codes, 1.0,
                                    self.guards)
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
                        act[side] = pair
                        act[1 - side] = their_pair
                        cell_world.append(k)
                        cell_actions.append(act)
                        cell_index.append((k, i, j))
                        seeds.append(seed)
        trackers = [(encoder.snapshot(int(2 * e)), encoder.snapshot(int(2 * e + 1))) for e in envs_s]
        out = self.grid.run(battles, np.stack(cell_actions), trackers, side,
                            world=np.asarray(cell_world, dtype=np.int64), seeds=np.stack(seeds).astype(np.int64),
                            allow_live=True)  # the true battle on purpose (module docstring)
        n = len(cell_actions)
        values = np.zeros(n, dtype=np.float32)
        live = np.flatnonzero(out["eval_side"] >= 0)
        if len(live):
            critic = model if self.value_model is None else self.value_model
            leaf = critic.trunk(*(torch.from_numpy(out[key][live]) for key in ("ids", "floats", "field")),
                                torch.zeros(len(live), 2, 40, 0), torch.from_numpy(out["token_features"][live]))
            v = critic.value(leaf, torch.from_numpy(out["partner_ids"][live]),
                             torch.from_numpy(out["partner_floats"][live])).clamp(-1, 1).numpy()
            values[live] = np.where(out["eval_side"][live] == side, v, -v)
        done = out["ended"]
        winner = out["winner"]
        values[done] = np.where(winner[done] == side, 1.0, np.where(winner[done] < 0, 0.0, -1.0))
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
                step = 0 if self.steps is None else int(self.steps[e])
                self.pending.setdefault(int(e), []).append(
                    (self._label(arrays, int(rows[k]), own[k], ours[k], x[k], q[k]), minutes, teacher, step, side))
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
        partner = row ^ 1  # the other side of the same env: its own view (train_selfplay.partner_view)
        row_arrays.update(mask0=mask0[0], mask1=mask1, pairs=padded, x=weights, q=values, prior=prior,
                          value=np.float32(float(w @ q[:n])), partner_ids=arrays["ids"][partner].copy(),
                          partner_floats=arrays["floats"][partner, 0:6].copy(),
                          foe_match=arrays["foe_match"][row].copy())
        return row_arrays
