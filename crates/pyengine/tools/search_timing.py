"""Per-decision time of a search bot on stored positions, with the ladder's time rules.

    cd crates/pyengine
    PYTHONPATH=. nice -n 10 .venv/bin/python tools/search_timing.py --bot-config play/ladder-mac.json \\
        --checkpoint ../../scratch/play-runtime/models/gen1/numbrion-spark.pt --set-pool POOL \\
        --positions scratch/positions.jsonl --limit 300 --out scratch/timing.json

Positions come from ``search_eval.py --positions-out`` (JSONL, tools/positions.py) or, with ``--generate N``, from N
self-play games of the plain policy (sampled at ``--generate-rate``). Each position is replayed into a fresh player
of the bot (``record_choice`` for its past choices, the world builder warmed as in a live game), then the player
decides once, timed from its last lines and request, as play/driver.py runs it: ``budget_ms`` = min(``search_s``,
time to the deadline - 250 ms) and ``deadline`` = now + ``deadline_s`` - ``reserve_s`` (``timing`` of a ladder
config, or ``--search-s``/``--deadline-s``/``--reserve-s``).

One decision at a time, in this thread: the numbers are what one ladder battle sees on an idle machine. Run it with
``nice`` and nothing else heavy running.

Reported: decision time (mean, p50, p90, p99, max), the share over ``--threshold-ms`` (default 5000) and past the
deadline, decision sources, and per-stage times from the analysis (``timing``; ``depth2.timing`` for the second layer).
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]  # crates/pyengine
sys.path.insert(0, str(HERE))
REPO = HERE.parents[1]

import numpy as np  # noqa: E402

from tools.positions import ReplayError, derive_seed, read_positions, replay_position  # noqa: E402

THINK_MARGIN_MS = 250.0  # play/driver.py


def load_bot_config(args) -> tuple[dict, dict]:
    """(bot kwargs, timing) from ``--bot-config`` (a ladder/bots config with ``bot`` and ``timing``, or a scenario
    config with ``search``) and the command-line overrides."""
    bot, timing = {}, {}
    if args.bot_config:
        config = json.loads(Path(args.bot_config).read_text())
        if "bot" in config:
            bot, timing = dict(config["bot"]), dict(config.get("timing") or {})
        elif "bots" in config:
            bot = dict(config["bots"][0])
            timing = dict(config.get("timing") or {})
        else:  # a scenario/eval config: {"name", "search", "budget", "budget_ms", "deadline_s", ...}
            bot = {k: v for k, v in config.items() if k not in ("budget_ms", "deadline_s", "reserve_s")}
            if config.get("budget_ms") is not None:
                timing["search_s"] = config["budget_ms"] / 1000.0
            for key in ("deadline_s", "reserve_s"):
                if config.get(key) is not None:
                    timing[key] = config[key]
    if args.search:
        bot["search"] = json.loads(args.search)
    if args.checkpoint:
        bot["checkpoint"] = args.checkpoint
    if args.set_pool:
        bot["pools"] = [args.set_pool]
    if args.device:
        bot["device"] = args.device
    for key in ("search_s", "deadline_s", "reserve_s"):
        value = getattr(args, key)
        if value is not None:
            timing[key] = None if value < 0 else value
    timing.setdefault("search_s", 3.0)
    timing.setdefault("deadline_s", 6.0)
    timing.setdefault("reserve_s", 0.5)
    for key in ("factory", "name"):
        bot.pop(key, None)
    if "checkpoint" not in bot:
        raise SystemExit("no checkpoint: give --checkpoint or a --bot-config with one")
    bot["checkpoint"] = str(resolve(bot["checkpoint"]))
    bot["pools"] = [str(resolve(p)) for p in bot.get("pools") or [REPO / "data/teams/pool-s1-200k.txt.gz"]]
    return bot, timing


def resolve(path) -> Path:
    path = Path(path).expanduser()
    return path if path.is_absolute() else (REPO / path)


def generate_positions(bot, games: int, rate: float, seed: int, teams_path: str, temperature: float) -> list[dict]:
    """Decision points of ``games`` games of the base policy against itself (search_eval.BasePolicy)."""
    import search_eval

    teams = [line.strip() for line in Path(teams_path).read_text().splitlines() if line.strip()]
    rng = np.random.default_rng(seed)
    dex_path = str(bot.dex.path)
    positions = []
    for g in range(games):
        i, j = rng.choice(len(teams), size=2, replace=False)
        battle_seed = tuple(int(x) for x in rng.integers(0, 1 << 16, size=4))
        game_rng = np.random.default_rng([seed, g])
        factory = lambda side, team, seed=None: search_eval.BasePolicy(bot.net, dex_path, side,  # noqa: E731
                                                                         temperature, game_rng)

        result = play_policy_game(bot, factory, teams[i], teams[j], battle_seed, rate,
                                  np.random.default_rng([seed, g, 7]))
        for position in result:
            positions.append(dict(position, game=g))
    return positions


def play_policy_game(bot, factory, team1, team2, battle_seed, rate, rng, max_turns=200) -> list[dict]:
    """One base-policy self-play game; returns the sampled positions (the format of tools/positions.py)."""
    import numbrion

    battle = numbrion.Battle(battle_seed, p1=team1, p2=team2, log=True)
    battle.start()
    players = [factory(0, team1), factory(1, team2)]
    history, positions = [], []
    while not battle.ended and battle.turn <= max_turns:
        lines = [battle.drain_player_log(s) for s in (0, 1)]
        for s in (0, 1):
            players[s].feed(lines[s])
        for side in [s for s in (0, 1) if battle.needs_action(s)]:
            if rng.random() < rate:
                positions.append(dict(teams=[team1, team2], seed=list(battle_seed), names=None, side=side,
                                      turn=int(battle.turn), role="policy", request_kind=battle.request_kind(side),
                                      history=[dict(h) for h in history]))
            a0, a1 = players[side].act(battle)
            battle.choose_action(side, a0, a1)
            history.append(dict(side=side, a0=int(a0), a1=int(a1)))
    return positions


def stage_stats(rows: list[dict]) -> dict:
    """Mean and p90 of every numeric stage time across decisions (keys missing in a decision count as absent)."""
    values: dict = {}
    for row in rows:
        for key, value in (row or {}).items():
            if isinstance(value, (int, float)) and not isinstance(value, bool):
                values.setdefault(key, []).append(float(value))
    return {key: dict(n=len(v), mean=round(float(np.mean(v)), 1), p90=round(float(np.percentile(v, 90)), 1))
            for key, v in sorted(values.items())}


def time_stats(values) -> dict | None:
    values = np.asarray(values, dtype=float)
    if not len(values):
        return None
    p50, p90, p99 = np.percentile(values, [50, 90, 99])
    return dict(count=int(len(values)), mean=round(float(values.mean()), 1), p50=round(float(p50), 1),
                p90=round(float(p90), 1), p99=round(float(p99), 1), max=round(float(values.max()), 1))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bot-config", default=None,
                        help="ladder config (bot + timing, e.g. play/ladder-mac.json) or a scenario config")
    parser.add_argument("--checkpoint", default=None, help="overrides the config's checkpoint")
    parser.add_argument("--set-pool", default=None, help="overrides the config's pools")
    parser.add_argument("--search", default=None, help="SearchConfig JSON, overrides the config's search")
    parser.add_argument("--device", default=None)
    parser.add_argument("--search-s", type=float, default=None, help="search allowance (s); <0: none")
    parser.add_argument("--deadline-s", type=float, default=None, help="deadline from receipt (s); <0: none")
    parser.add_argument("--reserve-s", type=float, default=None)
    parser.add_argument("--positions", default=None, help="JSONL from search_eval.py --positions-out")
    parser.add_argument("--generate", type=int, default=0, help="or: positions from this many policy games")
    parser.add_argument("--generate-rate", type=float, default=0.15)
    parser.add_argument("--teams", default=str(REPO / "data/teams/eval-s43-200.txt"), help="teams for --generate")
    parser.add_argument("--save-positions", default=None, help="write the generated positions here (JSONL)")
    parser.add_argument("--limit", type=int, default=300, help="at most this many positions")
    parser.add_argument("--warmup", type=int, default=3, help="decisions run first and not counted")
    parser.add_argument("--threshold-ms", type=float, default=5000.0)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--nice", type=int, default=10, help="raise the process niceness by this much")
    parser.add_argument("--out", default=None, help="JSON report with every decision")
    args = parser.parse_args()
    if args.nice:
        os.nice(args.nice)

    import torch

    from training.search import SearchBot

    bot_kwargs, timing = load_bot_config(args)
    torch.set_num_threads(int(bot_kwargs.get("torch_threads") or 4))
    bot = SearchBot(name="timing", **bot_kwargs)
    bot.warmup()
    if args.positions:
        positions = read_positions(args.positions)
    elif args.generate:
        positions = generate_positions(bot, args.generate, args.generate_rate, args.seed, args.teams, 0.5)
        if args.save_positions:
            Path(args.save_positions).write_text("".join(json.dumps(p) + "\n" for p in positions))
    else:
        raise SystemExit("give --positions FILE or --generate N")
    rng = np.random.default_rng(args.seed)
    if len(positions) > args.limit:
        positions = [positions[i] for i in sorted(rng.choice(len(positions), args.limit, replace=False))]
    print(f"{len(positions)} positions; timing {timing}; search {bot.describe()['search']}", flush=True)

    rows = []
    for k, position in enumerate(positions):
        player_seed = derive_seed(args.seed, k)
        try:
            replay, player = replay_position(position, bot, player_seed)
        except ReplayError as error:
            print(f"position {k}: does not replay: {error}", flush=True)
            continue
        # The driver's _think: the time counts from the request's receipt; feed and set_request already ran in
        # replay_position, so the budget is computed now, as the driver does right before decide().
        received = time.monotonic()
        deadline = None
        if timing.get("deadline_s") is not None:
            deadline = received + timing["deadline_s"] - timing.get("reserve_s", 0.5)
        budget = None if timing.get("search_s") is None else timing["search_s"] * 1000.0
        if deadline is not None:
            left = (deadline - time.monotonic()) * 1000.0 - THINK_MARGIN_MS
            budget = left if budget is None else min(budget, left)
        if budget is not None or deadline is not None:
            player.budget_ms = max(budget, 1.0)
            player.deadline = deadline
        player.decision_token = ("timing", k)
        started = time.monotonic()
        decision = player.decide()
        finished = time.monotonic()
        analysis = (decision.analysis if decision is not None else {}) or {}
        depth2 = analysis.get("depth2") if isinstance(analysis.get("depth2"), dict) else None
        row = dict(index=k, turn=position.get("turn"), side=position.get("side"), ms=round((finished - received) * 1000, 1),
                   decide_ms=round((finished - started) * 1000, 1), source=analysis.get("source"),
                   worlds=analysis.get("worlds"), cells=analysis.get("cells"), budget_ms=budget,
                   past_deadline=deadline is not None and finished > deadline,
                   timing=analysis.get("timing"), depth2_timing=(depth2 or {}).get("timing"),
                   depth2=None if depth2 is None else {key: depth2.get(key) for key in
                                                       ("rounds", "chosen", "changed", "skipped", "total_ms")
                                                       if key in depth2})
        player.close()
        if k < args.warmup:
            continue
        rows.append(row)
        if len(rows) % 25 == 0:
            ms = [r["ms"] for r in rows]
            print(f"{len(rows)} decisions: mean {np.mean(ms):.0f} ms, p90 {np.percentile(ms, 90):.0f}, "
                  f"max {max(ms):.0f}", flush=True)

    ms = [r["ms"] for r in rows]
    searched = [r["ms"] for r in rows if r["source"] == "search"]
    sources: dict = {}
    for r in rows:
        sources[r["source"]] = sources.get(r["source"], 0) + 1
    summary = dict(decisions=len(rows), all=time_stats(ms), searched=time_stats(searched),
                   over_threshold=round(float(np.mean([m > args.threshold_ms for m in ms])), 4) if ms else None,
                   threshold_ms=args.threshold_ms,
                   past_deadline=sum(r["past_deadline"] for r in rows), sources=sources,
                   worlds_mean=round(float(np.mean([r["worlds"] for r in rows if r["worlds"]])), 2)
                   if any(r["worlds"] for r in rows) else None,
                   stages=stage_stats([r["timing"] for r in rows]),
                   depth2_stages=stage_stats([r["depth2_timing"] for r in rows if r["depth2_timing"]]) or None)
    print(json.dumps(summary, indent=1))
    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(dict(args=vars(args), bot=bot.describe(), timing=timing, summary=summary,
                                       decisions=rows), indent=1, default=str))


if __name__ == "__main__":
    main()
