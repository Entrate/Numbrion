"""Fast label-only generator: the training actors' shallow one-turn search (training.search_opponent) labels every
searched decision, with no learning step.

    .venv/bin/python crates/pyengine/shallow_labels.py --checkpoint TEACHER.pt --value-checkpoint VALUE.pt \\
        --workers 7 --envs 112 --minutes 600 --out scratch/NAME

Every env of training.actors.Actors is a SEARCH env and BOTH sides search (``Actors(search_both=True)``): the
shallow search (``--ours`` x ``--theirs`` pairs, ``--seeds``) with the teacher's candidates and priors, and leaf
values from ``--value-checkpoint`` (any size) or else the teacher's own value head. Each side's search hides the
other side's unseen bench. Position variety comes from ``--explore`` (as search_labels.py): with that probability a
searched decision PLAYS a pair sampled from the teacher policy at temperature 1, but its label is still the search's
strategy. The actors loop ``--steps``-boundary rollouts on the CPU (one torch thread each); nothing is trained.

Sleep labels (``--sleep-explore P``, default 0 = off, bit-identical rows): on a searched decision where a slot of the
acting side has a legal ``--sleep-moves`` move with an effective target (training.sleep documents the checks), with
probability P that slot plays the sleep move and its partner the search's best compatible part (see
training.search_opponent). The decision is still labelled with the search's strategy (the forced pair is never a
policy target); it takes precedence over ``--explore`` when both fire. With P > 0 every row has ``sleep_forced`` and
every shard the scalar ``sleep_explore`` (readers before this change reject such shards). Status lines count
``sleep_forced`` / ``sleep_eligible`` decisions, inflicted ``sleep_onsets`` (Rest counted apart), finished
``games_with_sleep`` and ``sleeps_per_1k_labels``.

Each eligible searched decision of either side becomes one row of search_labels.py's format (obs, masks, pairs, x, q, prior, value = x.q,
partner_ids / partner_floats / foe_match, outcome and remaining_steps), readable by distill_train.py and
training.distill. A row is written when its battle ends; at shutdown the rows of battles still running are written as
capped (NaN outcome, -1 steps: policy targets without a value target). Decisions whose exact masks differ from the
player's request, or whose unseen opponent bench cannot be fully swapped, are excluded (``rejected_labels``).
Each actor writes its own
``labels-<run>-<seq>.npz`` shards (``--shard`` rows, atomic, every 20th a holdout shard) into ``--out``.

Information: each search plays the true battle, so it knows the hidden moves, item, ability, Tera type and stats of
the opponent's Pokemon it has seen (the unseen bench is swapped for pool sets). See training/search_opponent.py.

Stops at ``--minutes``, ``--labels`` (rows of finished battles), SIGINT / SIGTERM to the parent, or when ``<out>/STOP`` exists, after the current
rollout. ``--follow DIR`` reloads the newest teacher of DIR (search_labels.newest_teacher) between rollouts; the
value checkpoint stays fixed. Runs at ``--nice`` priority (the actors inherit it).
"""
from __future__ import annotations

import argparse
import json
import os
import pickle
import signal
import sys
import time
from contextlib import contextmanager
from dataclasses import asdict, fields
from pathlib import Path

import numpy as np
import torch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
REPO = HERE.parents[1]

from search_labels import Teacher, follow_teacher, output_lock  # noqa: E402
from training.actors import Actors  # noqa: E402
from training.dex import load_dex  # noqa: E402
from training.league import SEARCH  # noqa: E402
from training.model import Model  # noqa: E402
from training.search_opponent import OpponentSearch  # noqa: E402
from training.sleep import DEFAULT_SLEEP_MOVES  # noqa: E402


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--checkpoint", type=Path, help="the teacher policy")
    source.add_argument("--follow", type=Path, help="directory of teacher checkpoints: the newest, reloaded between rollouts")
    parser.add_argument("--value-checkpoint", type=Path, default=None,
                        help="fixed leaf-value network (any size); default: the teacher's own value head")
    parser.add_argument("--workers", type=int, default=6, help="actor processes (one CPU core each)")
    parser.add_argument("--envs", type=int, default=96, help="battles over all actors")
    parser.add_argument("--steps", type=int, default=32, help="boundaries per rollout (status / STOP / reload granularity)")
    parser.add_argument("--minutes", type=float, default=60.0, help="start no rollout after this (inf: until --labels or STOP)")
    parser.add_argument("--labels", type=int, default=0, help="stop once this many rows are finished (0: no target)")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--shard", type=int, default=2048, help="rows per shard file (per actor)")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--pool", type=Path, default=REPO / "data/teams/train-s42-2000.txt",
                        help="teams of the battles; also the pool sets that hide the policy side's unseen bench")
    defaults = OpponentSearch()
    parser.add_argument("--ours", type=int, default=defaults.ours, help="our candidate pairs")
    parser.add_argument("--theirs", type=int, default=defaults.theirs, help="opponent candidate pairs")
    parser.add_argument("--seeds", type=int, default=defaults.seeds, help="chance seeds per cell")
    parser.add_argument("--search", default="{}", help="other OpponentSearch fields as JSON")
    parser.add_argument("--explore", type=float, default=0.25,
                        help="share of searched decisions that play a teacher sample at temperature 1 (label: the search's)")
    parser.add_argument("--sleep-explore", type=float, default=0.0,
                        help="share of sleep-eligible searched decisions that play a sleep move (label: the search's)")
    parser.add_argument("--sleep-moves", default=",".join(DEFAULT_SLEEP_MOVES),
                        help="comma-separated move ids that count as sleep moves")
    parser.add_argument("--nice", type=int, default=10, help="added niceness (0: none)")
    parser.add_argument("--status-seconds", type=float, default=45.0)
    args = parser.parse_args(argv)
    search = json.loads(args.search)
    unknown = set(search) - {f.name for f in fields(OpponentSearch)}
    if unknown:
        parser.error(f"unknown OpponentSearch fields: {sorted(unknown)}")
    args.search_config = asdict(OpponentSearch(**{**search, "ours": args.ours, "theirs": args.theirs,
                                                  "seeds": args.seeds}))
    if args.ours < 1 or args.theirs < 1 or args.seeds < 1:
        parser.error("--ours, --theirs and --seeds must be positive")
    if args.ours > 8:
        parser.error("--ours is at most 8 (the label width, training.search_opponent.LABEL_PAIRS)")
    if not 0 <= args.explore <= 1:
        parser.error("--explore must be in [0, 1]")
    if not 0 <= args.sleep_explore <= 1:
        parser.error("--sleep-explore must be in [0, 1]")
    args.sleep_move_ids = [m.strip() for m in args.sleep_moves.split(",") if m.strip()]
    if args.sleep_explore > 0 and not args.sleep_move_ids:
        parser.error("--sleep-explore needs at least one --sleep-moves id")
    if args.workers < 1 or args.envs < args.workers or args.steps < 1 or args.shard < 1 or not args.minutes > 0:
        parser.error("workers, envs >= workers, steps, shard and minutes must be positive")
    return args


def load_model(path: Path) -> tuple[Model, dict]:
    state = torch.load(path, map_location="cpu", weights_only=False)
    return Model.from_state(load_dex(), state["model"], state.get("width", 128), state.get("layers", 3)).eval(), state


def model_shape(model: Model) -> tuple:
    return (model.width, len(model.blocks), model.joint_features, model.n_action_features, model.n_token_features)


@contextmanager
def stop_signals():
    """SIGINT / SIGTERM stop after the active rollout, so actors can save capped rows."""
    requested = []

    def stop(signum, frame):
        requested.append(signal.Signals(signum).name)

    previous = {s: signal.signal(s, stop) for s in (signal.SIGINT, signal.SIGTERM)}
    try:
        yield requested
    finally:
        for s, handler in previous.items():
            signal.signal(s, handler)


def new_shards(out: Path, before: set) -> list[Path]:
    """This run's shards: the ones that were not in ``out`` when it started."""
    return [path for path in out.glob("labels-*.npz") if path.name not in before]


def disk_bytes(paths) -> int:
    total = 0
    for path in paths:
        try:
            total += path.stat().st_size
        except FileNotFoundError:
            pass
    return total


def cpu_seconds(processes) -> float:
    """User + system CPU seconds of the live actor processes (0 without psutil)."""
    try:
        import psutil
    except ImportError:
        return 0.0
    total = 0.0
    for process in processes:
        try:
            times = psutil.Process(process.pid).cpu_times()
            total += times.user + times.system
        except psutil.Error:
            pass
    return total


def main(argv=None) -> dict:
    args = parse_args(argv)
    with output_lock(args.out), stop_signals() as requested:
        return generate(args, requested)


def generate(args, requested=None) -> dict:
    if args.nice:
        os.nice(args.nice)
    torch.set_num_threads(1)
    (args.out / "config.json").write_text(json.dumps(vars(args), default=str, indent=1))
    teacher = follow_teacher(args.follow) if args.follow else Teacher.load(args.checkpoint)
    if teacher is None:
        raise FileNotFoundError(f"no finished teacher in {args.follow}")
    model, state = load_model(teacher.path)
    shape = model_shape(model)
    print(f"Teacher: {teacher.path.name} ({model.width}x{len(model.blocks)}), minutes={teacher.minutes:g}; "
          f"leaf values: {args.value_checkpoint.name if args.value_checkpoint else 'teacher'}; "
          f"search {args.search_config}, both sides, explore {args.explore}, sleep explore {args.sleep_explore}",
          flush=True)
    totals = dict(rollouts=0, decisions=0, forced=0, explored=0, labels=0, games=0, crashes=0, rejected_labels=0,
                  sleep_eligible=0, sleep_forced=0, sleep_onsets=0, rest_sleeps=0, games_with_sleep=0)
    started = time.monotonic()
    deadline = started + args.minutes * 60
    last_log, last_labels, last_time, last_cpu = started, 0, started, 0.0
    before = {path.name for path in args.out.glob("labels-*.npz")}
    actors = None

    def status(final=False):
        nonlocal last_log, last_labels, last_time, last_cpu
        now = time.monotonic()
        elapsed = max(now - started, 1e-6)
        cpu = cpu_seconds(actors.processes) if actors is not None and not final else last_cpu
        size = disk_bytes(new_shards(args.out, before))
        recent = (totals["labels"] - last_labels) / max(now - last_time, 1e-6)
        line = dict(minutes=round(elapsed / 60, 2), labels=totals["labels"], pending=totals.get("pending", 0),
                    labels_per_s=round(totals["labels"] / elapsed, 1), recent_labels_per_s=round(recent, 1),
                    decisions=totals["decisions"], explored=totals["explored"], rejected_labels=totals["rejected_labels"],
                    sleep_eligible=totals["sleep_eligible"], sleep_forced=totals["sleep_forced"],
                    sleep_onsets=totals["sleep_onsets"], rest_sleeps=totals["rest_sleeps"],
                    sleeps_per_1k_labels=round(1000 * totals["sleep_onsets"] / max(totals["labels"], 1), 2),
                    games_with_sleep=totals["games_with_sleep"],
                    games=totals["games"], rollouts=totals["rollouts"], crashes=totals["crashes"],
                    written=totals.get("written", 0), disk_mb=round(size / 2 ** 20, 1),
                    bytes_per_label=round(size / max(totals.get("written", 0), 1)), teacher=teacher.path.name,
                    actor_cores=None if final else round((cpu - last_cpu) / max(now - last_time, 1e-6), 2),
                    actor_rss_mb=totals.get("actor_rss_mb"), load=round(os.getloadavg()[0], 2), final=final)
        print("Status: " + " ".join(f"{k}={v}" for k, v in line.items()), flush=True)
        with (args.out / "status.jsonl").open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(line) + "\n")
        last_log, last_labels, last_time, last_cpu = now, totals["labels"], now, cpu

    def stopping():
        if requested:
            return requested[0]
        if time.monotonic() >= deadline:
            return "minutes"
        if args.labels and totals["labels"] >= args.labels:
            return "labels"
        if (args.out / "STOP").exists():
            return "STOP"
        return None

    newest = teacher  # the newest checkpoint --follow has seen (it may have been skipped)
    opponents = np.full(args.envs, SEARCH, dtype=np.int8)
    actors = Actors(args.envs, args.steps, [args.pool], args.seed, args.workers, model, search=args.search_config,
                    search_labels=args.out, search_value=args.value_checkpoint, search_shard=args.shard,
                    search_both=True, search_explore=args.explore, search_fair_labels=True, label_only=True,
                    search_sleep_explore=args.sleep_explore, search_sleep_moves=args.sleep_move_ids)
    try:
        actors.set_weights("learner", state["model"])
        actors.set_weights("opponent", state["model"])  # unused (no SNAPSHOT envs), but never uninitialised
        del state
        while (reason := stopping()) is None:
            if args.follow:
                replacement = follow_teacher(args.follow, newest)
                if replacement is not newest:
                    try:
                        new_model, new_state = load_model(replacement.path)
                    except (OSError, EOFError, RuntimeError, ValueError, KeyError, pickle.UnpicklingError) as error:
                        # Pruning / replacement can race the metadata load. Retry on the next rollout.
                        print(f"Teacher skipped: {replacement.path.name}: {error}", flush=True)
                    else:
                        newest = replacement  # valid wrong-shape files need only be inspected once
                        if model_shape(new_model) != shape:
                            print(f"Teacher skipped: {replacement.path.name} has another shape than the actors' model",
                                  flush=True)
                        else:
                            actors.set_weights("learner", new_state["model"])
                            print(f"Teacher switch: {teacher.path.name} -> {replacement.path.name}, "
                                  f"minutes={replacement.minutes:g}", flush=True)
                            teacher = replacement
                        del new_model, new_state
            actors.start(0, opponents, 0.0, minutes=teacher.minutes, label_teacher=teacher.path.name)
            collected = actors.wait()
            totals["rollouts"] += 1
            totals["decisions"] += collected["search_decisions"]
            totals["forced"] += collected["search_forced"]
            totals["explored"] += collected["search_explored"]
            totals["rejected_labels"] += collected.get("search_rejected_labels", 0)
            totals["sleep_eligible"] += collected.get("search_sleep_eligible", 0)
            totals["sleep_forced"] += collected.get("search_sleep_forced", 0)
            for key in ("sleep_onsets", "rest_sleeps", "games_with_sleep"):
                totals[key] += collected.get(key, 0)
            totals["labels"] += collected["search_labels"]
            totals["games"] += collected["completed"]
            totals["crashes"] += len(collected["crashes"])
            if collected["crashes"]:
                with (args.out / "crashes.jsonl").open("a", encoding="utf-8") as stream:
                    for crash in collected["crashes"]:
                        stream.write(json.dumps(crash) + "\n")
            totals["pending"] = collected["search_pending"]
            totals["written"] = collected["search_written"]
            totals["actor_rss_mb"] = collected.get("actor_rss_mb")
            if time.monotonic() - last_log >= args.status_seconds:
                status()
        print(f"Stopping ({reason}): writing the buffered rows; running battles' rows as capped", flush=True)
    finally:
        closed = actors.close()
    totals["capped_at_close"] = closed["search_capped_at_close"]
    totals["pending"] = 0
    totals["written"] = closed["search_written"]
    status(final=True)
    print("Done: " + json.dumps(totals), flush=True)
    return totals


if __name__ == "__main__":
    main()
