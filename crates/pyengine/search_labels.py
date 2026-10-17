"""Search labels for policy distillation (train_selfplay.py ``--distill``): search-vs-search games where every searched
decision is stored with the search's mixed strategy over its candidate pairs.

    .venv/bin/python crates/pyengine/search_labels.py --checkpoint BEST.pt --minutes 60 --out scratch/distill/NAME

Both sides of every game are SearchPlayers of ``--checkpoint`` (the default SearchConfig unless ``--search``), driven
in-process like search_eval.py: each sees only its own player-view lines and requests. A decision that searched
(not forced, not a policy fallback) becomes one label: the player's encoded observation row (training.encoder, the
same rows the policy reads), the guarded slot-a mask, the candidate pairs (slot a, slot b) with the guarded slot-b mask
of each pair's slot a, the search's strategy ``x`` and pair values ``q``, and the policy's log-prior of each pair.
Rows also carry the opponent's own-view ``partner_ids`` / ``partner_floats``, ``foe_match`` for auxiliary targets,
and the final ``outcome`` with ``remaining_steps`` for discounting. Capped games store NaN / -1 for the last two.

``--explore``: the share of decisions where the side plays a pair sampled from the policy at temperature 1 instead of
the search's choice (the label is still the search's), so the labelled positions also cover worse play than the
search's own. Atomic ``labels-<UTC run>-<sequence>.npz`` shards include teacher metadata. ``--follow DIR``
reloads newer teachers between games; ``--minutes inf`` runs until ``<out>/STOP``, then drains games and rows.
"""
from __future__ import annotations

import argparse
import gzip
import json
import os
import pickle
import re
import sys
import threading
import time
import uuid

if sys.platform == "win32":
    import msvcrt
else:
    import fcntl
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

import numpy as np
import torch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
REPO = HERE.parents[1]

import numbrion  # noqa: E402

from training.guards import MoveGuards  # noqa: E402
from training.encoder import PyEncoder  # noqa: E402
from training.protocol import choice_text  # noqa: E402
from training.search import N_ACTIONS, OBS_KEYS, SearchBot  # noqa: E402

GUARDS = MoveGuards()


def teacher_key(path: Path) -> tuple:
    match = re.fullmatch(r"m(\d+)\.pt", path.name)
    return (1, int(match[1]), path.stat().st_mtime_ns) if match else (0, path.stat().st_mtime_ns, path.name)


def newest_teacher(directory: Path) -> Path | None:
    """Numbered snapshots take precedence; other finished checkpoints fall back to modification time."""
    newest, version = None, ()
    for path in directory.glob("*.pt"):
        try:
            if not path.is_file():
                continue
            key = teacher_key(path)
        except FileNotFoundError:
            continue
        if key > version:
            newest, version = path, key
    return newest


@dataclass(frozen=True)
class Teacher:
    path: Path
    minutes: float
    version: tuple

    @classmethod
    def load(cls, path: Path):
        state = torch.load(path, map_location="cpu", weights_only=False)
        return cls(path, float(state.get("minutes", float("nan"))), teacher_key(path))


def follow_teacher(directory: Path, current: Teacher | None = None) -> Teacher | None:
    """Keep the current teacher when pruning or an incomplete checkpoint interrupts a reload."""
    path = newest_teacher(directory)
    if path is None:
        return current
    try:
        if current is not None and teacher_key(path) <= current.version:
            return current
        return Teacher.load(path)
    except (OSError, EOFError, RuntimeError, ValueError, pickle.UnpicklingError) as error:
        print(f"Teacher skipped: {path.name}: {error}", flush=True)
        return current


@contextmanager
def output_lock(out: Path, name: str = ".labels.lock", holder: str = "label maker"):
    """Hold a persistent lock inode until generation, including all worker games, has finished."""
    out.mkdir(parents=True, exist_ok=True)
    with (out / name).open("a") as lock:
        try:
            if sys.platform == "win32":  # locks the file's first byte; released when the process exits
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:  # BlockingIOError (flock) or a lock-violation OSError (msvcrt)
            raise RuntimeError(f"another {holder} holds {out / name}") from error
        yield


class ShardWriter:
    """Keep one teacher per shard, including late results from games that began before a reload."""

    def __init__(self, out: Path, size: int, run: str | None = None):
        self.out, self.size = out, size
        self.run = run or (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-%f") + "-" + uuid.uuid4().hex)
        if any(out.glob(f"labels-{self.run}-*.npz*")):
            raise FileExistsError(f"label run {self.run} already exists in {out}")
        self.pending, self.teacher, self.sequence = [], None, 0

    def add(self, rows: list[dict], teacher: Teacher):
        if not rows:
            return
        if self.teacher is not None and teacher.version != self.teacher.version:
            self.flush(final=True)
        self.teacher = teacher
        self.pending.extend(rows)
        self.flush()

    def flush(self, final=False):
        while len(self.pending) >= self.size or (final and self.pending):
            rows = self.pending[:self.size]
            arrays = {key: np.stack([row[key] for row in rows]) for key in rows[0]}
            arrays.update(teacher=np.asarray(self.teacher.path.name),
                          teacher_minutes=np.asarray(self.teacher.minutes, dtype=np.float64),
                          created=np.asarray(time.time(), dtype=np.float64),
                          holdout=np.asarray(self.sequence % 20 == 19, dtype=bool))
            path = self.out / f"labels-{self.run}-{self.sequence:06d}.npz"
            with path.with_suffix(".npz.tmp").open("wb") as stream:
                np.savez_compressed(stream, **arrays)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(path.with_suffix(".npz.tmp"), path)
            del self.pending[:len(rows)]
            self.sequence += 1


def read_teams(path: Path) -> list[str]:
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rt", encoding="utf-8") as stream:
        return [line.strip() for line in stream if line.strip()]


def make_label(search: dict, k: int) -> dict:
    """One label row from SearchPlayer.last_search, padded to ``k`` candidate pairs (pair -1, weight 0)."""
    side, arrays = search["side"], search["arrays"]
    guard = arrays["guard"][side:side + 1]
    mask0, _ = GUARDS.apply(search["mask0"][None], guard, 0)
    pairs = np.asarray(search["pairs"], dtype=np.int64).reshape(-1, 2)
    n = len(pairs)
    x = np.asarray(search["x"], dtype=np.float64)[:n]
    x = x / x.sum()
    mask1 = np.zeros((k, N_ACTIONS), dtype=bool)
    mask1[:n], _ = GUARDS.apply(search["joint"][pairs[:, 0]], np.repeat(guard, n, 0), 1)
    if not (mask0[0, pairs[:, 0]].all() and mask1[np.arange(n), pairs[:, 1]].all()):
        raise RuntimeError("a search candidate is illegal under the guarded masks")
    padded = np.full((k, 2), -1, dtype=np.int16)
    padded[:n] = pairs
    weights, values, prior = np.zeros(k, np.float32), np.zeros(k, np.float32), np.full(k, -30.0, np.float32)
    weights[:n], values[:n] = x, search["q"][:n]
    prior[:n] = [search["full"][(int(a), int(b))][0] for a, b in pairs]
    row = {key: arrays[key][side].copy() for key in OBS_KEYS}
    row.update(mask0=mask0[0], mask1=mask1, pairs=padded, x=weights, q=values, prior=prior,
               value=np.float32(float(x @ search["q"][:n])))
    return row


def play_game(bots, team1: str, team2: str, seed: tuple, explore: float, rng, k: int, max_turns: int = 400):
    battle = numbrion.Battle(seed, p1=team1, p2=team2, log=True)
    battle.start()
    players = []
    for side, (bot, team) in enumerate(zip(bots, (team1, team2))):
        player = bot.new_player(f"label-{seed}-{side}")
        player.team = team
        players.append(player)
    # Separate from the players' public opponent trackers: both rows get their OWN request, just like
    # Actors.observe's rows 2e / 2e+1. Encode before either choice can advance the battle.
    oracle = PyEncoder(2, bots[0].dex)
    labels, boundaries, explored, rejected, steps = [], [], 0, 0, 0
    while not battle.ended and battle.turn <= max_turns:
        lines = [battle.drain_player_log(s) for s in (0, 1)]
        requests = [battle.request_json(s) for s in (0, 1)]
        for side in (0, 1):
            oracle.encode(side, dict(battle_id=1, request=requests[side], log=lines[side]))
        oracle.match_partners()
        own = oracle.buffers  # no damage features: the critic's partner input and aux targets do not use them
        for side in (0, 1):
            players[side].feed(lines[side])
        for side in [s for s in (0, 1) if battle.needs_action(s)]:
            player = players[side]
            player.set_request(requests[side])
            for _ in range(3):
                decision = player.decide()
                if decision is None:
                    battle.choose(side, "default")
                    break
                a0, a1 = decision.a0, decision.a1
                if player.last_search is not None:
                    row = make_label(player.last_search, k)
                    row.update(partner_ids=own.ids[side ^ 1].copy(),
                               partner_floats=own.floats[side ^ 1, :6].copy(),
                               foe_match=own.foe_match[side].copy())
                    labels.append(row)
                    boundaries.append((side, steps))
                    if rng.random() < explore:
                        full = player.last_search["full"]
                        keys = list(full)
                        logp = np.array([full[key][0] for key in keys])
                        p = np.exp(logp - logp.max())
                        a0, a1 = keys[int(rng.choice(len(keys), p=p / p.sum()))]
                        text = choice_text(json.loads(battle.request_json(side)), a0, a1)
                        if text is not None and (a0, a1) != (decision.a0, decision.a1):
                            player.pending_choice = text
                            explored += 1
                        else:
                            a0, a1 = decision.a0, decision.a1
                try:
                    battle.choose_action(side, a0, a1)
                    break
                except numbrion.ChoiceError as error:
                    rejected += 1
                    player.rejected(str(error.args[0]))
                    if len(error.args) > 1 and error.args[1]:
                        player.set_request(error.args[1])
            else:
                battle.choose(side, "default")
        steps += 1
    winner = None if not battle.ended else (-1 if battle.tie or battle.winner is None else int(battle.winner))
    for row, (side, step) in zip(labels, boundaries):
        # The reward on the final transition is undiscounted in gae. Include forced-switch/wait boundaries,
        # not just battle turns or this side's searched decisions. A capped game has no terminal target.
        row["outcome"] = np.float32(np.nan if winner is None else 0 if winner < 0 else 1 if winner == side else -1)
        row["remaining_steps"] = np.int32(-1 if winner is None else steps - step - 1)
    stats = {key: sum(p.stats.get(key, 0) for p in players) for key in ("search", "policy", "forced", "no_worlds")}
    return labels, dict(turns=battle.turn, steps=steps, ended=battle.ended, winner=winner, labels=len(labels), explored=explored,
                        rejected=rejected, **stats)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--checkpoint", type=Path)
    source.add_argument("--follow", type=Path, help="directory of trainer checkpoints")
    parser.add_argument("--reload-minutes", type=float, default=20.0)
    parser.add_argument("--value-checkpoint", type=Path, default=None,
                        help="the search's leaf values (and so the labels' q/value) from this checkpoint (any size) "
                             "instead of the teacher's own value head")
    parser.add_argument("--search", default="{}", help="SearchConfig JSON")
    parser.add_argument("--pool", default=str(REPO / "data/teams/train-s42-2000.txt"), help="teams of the games")
    parser.add_argument("--set-pool", default=str(REPO / "data/teams/pool-s1-200k.txt.gz"), help="world builder pool")
    parser.add_argument("--minutes", type=float, default=60.0, help="start no game after this")
    parser.add_argument("--games", type=int, default=0, help="stop after this many games (0: --minutes only)")
    parser.add_argument("--explore", type=float, default=0.25)
    parser.add_argument("--policy-temperature", type=float, default=0.5, help="the players' fallback policy")
    parser.add_argument("--concurrency", type=int, default=4)
    parser.add_argument("--torch-threads", type=int, default=2)
    parser.add_argument("--device", default="auto")
    parser.add_argument("--shard", type=int, default=2000, help="labels per output file")
    parser.add_argument("--seed", type=int, default=11)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    if args.concurrency < 1 or args.shard < 1 or not args.reload_minutes > 0 or not args.minutes > 0:
        parser.error("concurrency, shard, reload-minutes and minutes must be positive")
    generate(args)


def generate(args) -> dict:
    with output_lock(args.out):
        return _generate(args)


def _generate(args) -> dict:
    torch.set_num_threads(args.torch_threads)
    search = json.loads(args.search)
    teams = read_teams(Path(args.pool))
    (args.out / "config.json").write_text(json.dumps(vars(args), default=str, indent=1))
    teacher = follow_teacher(args.follow) if args.follow else Teacher.load(args.checkpoint)
    if teacher is None:
        raise FileNotFoundError(f"no finished teacher in {args.follow}")
    local, lock = threading.local(), threading.Lock()
    writer = ShardWriter(args.out, args.shard)
    totals = dict(games=0, labels=0, shards=0)
    started = time.monotonic()
    deadline = started + args.minutes * 60
    next_reload, next_log = started + args.reload_minutes * 60, started + 30
    print(f"Teacher: {teacher.path.name}, minutes={teacher.minutes:g}", flush=True)

    def stopping():
        return time.monotonic() >= deadline or (args.out / "STOP").exists()

    def bots(version):
        if getattr(local, "version", None) != version.version:
            local.bots = [SearchBot(name=f"label{s}", checkpoint=str(version.path), search=search,
                                    pools=[args.set_pool], device=args.device,
                                    policy_temperature=args.policy_temperature,
                                    value_checkpoint=str(args.value_checkpoint) if args.value_checkpoint else None)
                          for s in (0, 1)]
            local.version = version.version
            for side, bot in enumerate(local.bots):
                bot.rng = np.random.default_rng([args.seed, threading.get_ident() % (1 << 31), side])
        return local.bots

    def run(index):
        with lock:
            if stopping():
                return None
            version = teacher
        game_rng = np.random.default_rng([args.seed, index])
        i, j = game_rng.choice(len(teams), size=2, replace=False)
        seed = tuple(int(x) for x in game_rng.integers(0, 1 << 16, size=4))
        labels, info = play_game(bots(version), teams[i], teams[j], seed, args.explore, game_rng,
                                 search.get("ours") or 8)
        return index, labels, info, version

    def status():
        elapsed = max(time.monotonic() - started, 1e-6)
        totals["shards"] = writer.sequence
        print(f"Status: games={totals['games']} labels={totals['labels']} "
              f"labels/s={totals['labels'] / elapsed:.1f} shards={writer.sequence} "
              f"teacher={teacher.path.name} minutes={elapsed / 60:.1f}", flush=True)

    limit, index, futures = args.games or 10 ** 18, 0, set()
    with ThreadPoolExecutor(args.concurrency) as pool:
        while futures or (index < limit and not stopping()):
            now = time.monotonic()
            if args.follow and now >= next_reload and not stopping():
                next_reload = now + args.reload_minutes * 60
                replacement = follow_teacher(args.follow, teacher)
                if replacement is not teacher:
                    with lock:
                        writer.flush(final=True)
                        print(f"Teacher switch: {teacher.path.name} -> {replacement.path.name}, "
                              f"minutes={replacement.minutes:g}", flush=True)
                        teacher = replacement
            while len(futures) < args.concurrency and index < limit and not stopping():
                futures.add(pool.submit(run, index))
                index += 1
            done, futures = wait(futures, timeout=0.2, return_when=FIRST_COMPLETED)
            for future in done:
                result = future.result()
                if result is None:
                    continue
                game, labels, info, version = result
                writer.add(labels, version)
                totals["games"] += 1
                totals["labels"] += len(labels)
                with (args.out / "games.jsonl").open("a", encoding="utf-8") as stream:
                    stream.write(json.dumps(dict(game=game, teacher=version.path.name,
                                                 teacher_minutes=version.minutes, **info)) + "\n")
            if now >= next_log:
                status()
                next_log = now + 30
    writer.flush(final=True)
    status()
    print("Done: in-flight games finished and pending rows flushed", flush=True)
    return totals


if __name__ == "__main__":
    main()
