"""Run the scenario suite (library.py) for one or more search configs and report each config's choices.

    cd crates/pyengine
    PYTHONPATH=. nice -n 10 .venv/bin/python tools/scenarios/run.py \\
        --config tools/scenarios/configs/1ply.json --config my-2ply.json --repeats 4 --out scratch/scenarios.json

Per scenario and config, the script is replayed ``--repeats`` times with a fresh, differently seeded player (the
same seeds for every config), feeding it exactly as search_eval's play_game does (player-view lines, the request
before each of its choices; past choices via ``record_choice``), and ``decide()`` is called once at the target.
Reported: the choice distribution, the pass rate against the scenario's expectation (informational scenarios have
none), the layer-1 value of the chosen pair, the second layer's values when the analysis has a ``depth2`` block, and
the decision time.

A config is JSON: ``{"name": "1ply", "search": {...SearchConfig, may include "depth2"...}, "budget": {...},
"budget_ms": null, "deadline_s": null, "policy_temperature": 0.5, "device": "cpu", "torch_threads": 2,
"cell_ms": 0.3}``. Only ``search`` is required. With ``budget_ms``/``deadline_s`` the player gets them as
play/driver.py hands them out; leave them out for fixed-size comparisons.

Authoring helpers: ``--check`` replays every selected scenario and prints its position (no network), ``--probe
NAME --p1 TEXT --p2 TEXT`` plays one turn from the target under several battle seeds.
"""
from __future__ import annotations

import argparse
import fnmatch
import json
import sys
import time
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parents[2]  # crates/pyengine
sys.path.insert(0, str(HERE))
REPO = HERE.parents[1]

import numpy as np  # noqa: E402

from tools.positions import derive_seed, new_player  # noqa: E402
from tools.scenarios.core import ScenarioError, build, describe_position, probe, view_of  # noqa: E402
from tools.scenarios.library import SCENARIOS  # noqa: E402

DEFAULT_CHECKPOINT = REPO / "scratch/play-runtime/models/gen1/numbrion-spark.pt"
DEFAULT_SET_POOL = REPO / "data/teams/pool-s1-200k.txt.gz"
THINK_MARGIN_MS = 250.0  # play/driver.py: kept back from the time left to the deadline


def load_config(path: str) -> dict:
    config = json.loads(Path(path).read_text())
    config.setdefault("name", Path(path).stem)
    if "search" not in config:
        raise SystemExit(f"{path}: a scenario config needs a 'search' entry")
    return config


def make_bot(config: dict, checkpoint: str, set_pool: str):
    from training.search import SearchBot

    kwargs = dict(name=config["name"], checkpoint=checkpoint, search=config["search"], pools=[set_pool],
                  device=config.get("device", "cpu"), policy_temperature=config.get("policy_temperature", 0.5),
                  torch_threads=config.get("torch_threads"), cell_ms=config.get("cell_ms", 0.3))
    if config.get("budget") is not None:
        kwargs["budget"] = config["budget"]
    bot = SearchBot(**kwargs)
    search = config["search"]
    if isinstance(search, dict) and "depth2" in search and bot.search is not None \
            and not hasattr(bot.search, "depth2"):
        raise SystemExit(f"config {config['name']}: 'depth2' is set but SearchConfig has no depth2 field "
                         "(training/search.py without the search2 core change)")
    return bot


def select(patterns: list[str] | None, families: list[str] | None):
    chosen = [s for s in SCENARIOS
              if (not patterns or any(fnmatch.fnmatch(s.name, p) for p in patterns))
              and (not families or s.family in families)]
    if not chosen:
        raise SystemExit("no scenario matches")
    return chosen


def depth2_values(analysis: dict) -> dict | None:
    """The second layer's view of the decision (training/search2.py's analysis block), tolerant of its exact keys."""
    block = analysis.get("depth2")
    if not isinstance(block, dict):
        return None
    out = {k: block[k] for k in ("rounds", "rounds_planned", "chosen", "changed", "skipped", "total_ms")
           if k in block}
    out["ran"] = bool(block.get("rounds")) and block.get("chosen") is not None
    if isinstance(block.get("timing"), dict):
        out["timing"] = block["timing"]
    if isinstance(block.get("candidates"), list):  # layer-1 x and q, layer-2 q2 per finished round
        out["candidates"] = [{k: c.get(k) for k in ("choice", "x", "q", "q2", "score") if k in c}
                             for c in block["candidates"] if isinstance(c, dict)]
    return out


def decide_once(scenario, bot, seed: int, config: dict) -> dict:
    player, seeded = new_player(bot, f"scenario-{scenario.name}", seed)
    if not seeded:  # bots without per-player seeds: reseed the bot (this runner is single-threaded)
        bot.rng = np.random.default_rng(seed)
    replay = build(scenario, player)
    request = json.loads(replay.battle.request_json(scenario.side))
    if config.get("budget_ms") is not None or config.get("deadline_s") is not None:
        deadline = None
        if config.get("deadline_s") is not None:
            deadline = time.monotonic() + config["deadline_s"] - config.get("reserve_s", 0.5)
        budget = config.get("budget_ms")
        if deadline is not None:
            left = (deadline - time.monotonic()) * 1000.0 - THINK_MARGIN_MS
            budget = left if budget is None else min(budget, left)
        player.budget_ms = max(budget, 1.0)
        player.deadline = deadline
    started = time.perf_counter()
    decision = player.decide()
    ms = (time.perf_counter() - started) * 1000
    if decision is None:
        return dict(seed=seed, choice=None, tokens=None, passed=False if scenario.expect else None, ms=ms)
    view = view_of(decision, request)
    analysis = decision.analysis or {}
    return dict(seed=seed, choice=view.text, tokens=list(view.tokens), readable=view.readable,
                passed=scenario.expect.check(view) if scenario.expect else None, source=analysis.get("source"),
                value=analysis.get("value"), worlds=analysis.get("worlds"), ms=round(ms, 1),
                timing=analysis.get("timing"), depth2=depth2_values(analysis),
                candidates=[{k: c.get(k) for k in ("choice", "text", "value", "weight")}
                            for c in (analysis.get("candidates") or [])[:5]])


def summarize(runs: list[dict], expect) -> dict:
    tokens = Counter(" | ".join(r["tokens"]) if r["tokens"] else "none" for r in runs)
    passed = [r["passed"] for r in runs if r["passed"] is not None]
    values = [r["value"] for r in runs if r.get("value") is not None]
    return dict(n=len(runs), choices=dict(tokens.most_common()),
                pass_rate=None if expect is None else (sum(passed) / len(runs) if runs else None),
                value_mean=round(float(np.mean(values)), 4) if values else None,
                ms_mean=round(float(np.mean([r["ms"] for r in runs])), 1),
                ms_max=round(float(np.max([r["ms"] for r in runs])), 1),
                depth2_ran=sum(bool(r["depth2"] and r["depth2"]["ran"]) for r in runs),
                depth2_changed=sum(bool(r["depth2"] and r["depth2"].get("changed")) for r in runs))


def table(report: list[dict], configs: list[str]) -> str:
    rows = [f"{'scenario':<34} {'family':<8} {'config':<12} {'pass':>5}  {'value':>6} {'ms':>6}  choices"]
    for entry in report:
        for name in configs:
            s = entry["configs"][name]["summary"]
            rate = "info" if s["pass_rate"] is None else f"{s['pass_rate']:.0%}"
            value = f"{s['value_mean']:+.2f}" if s["value_mean"] is not None else "   n/a"
            choices = "; ".join(f"{k} x{v}" for k, v in s["choices"].items())
            rows.append(f"{entry['name']:<34} {entry['family']:<8} {name:<12} {rate:>5}  {value:>6} "
                        f"{s['ms_mean']:>6.0f}  {choices}")
    return "\n".join(rows)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--config", action="append", default=[], help="config JSON (repeatable)")
    parser.add_argument("--checkpoint", default=str(DEFAULT_CHECKPOINT))
    parser.add_argument("--set-pool", default=str(DEFAULT_SET_POOL))
    parser.add_argument("--repeats", type=int, default=4, help="decisions per scenario and config (R)")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--scenario", action="append", default=None, help="name pattern (repeatable)")
    parser.add_argument("--family", action="append", default=None)
    parser.add_argument("--out", default=None, help="JSON report")
    parser.add_argument("--check", action="store_true", help="only replay the scenarios and print the positions")
    parser.add_argument("--probe", default=None, help="scenario name: play --p1/--p2 from its target")
    parser.add_argument("--p1", default=None)
    parser.add_argument("--p2", default=None)
    parser.add_argument("--probe-n", type=int, default=10)
    parser.add_argument("--find-seed", type=int, default=0,
                        help="authoring: try this many battle seeds and list those that reach the target")
    args = parser.parse_args()

    if args.find_seed:
        from dataclasses import replace

        for scenario in select(args.scenario, args.family):
            found = []
            for i in range(args.find_seed):
                seed = tuple(int(x) for x in np.random.default_rng([args.seed, i]).integers(0, 1 << 16, size=4))
                try:
                    build(replace(scenario, seed=seed))
                except ScenarioError:
                    continue
                found.append(seed)
            print(f"{scenario.name}: {len(found)}/{args.find_seed} seeds reach the target: {found[:8]}")
        return

    if args.probe:
        scenario = select([args.probe], None)[0]
        for k, outcome in enumerate(probe(scenario, args.p1, args.p2, args.probe_n, args.seed)):
            states = ", ".join(f"p{s + 1} {sp} {st['condition']}" for (s, sp), st in outcome["states"].items()
                               if st["active"] or st["status"] == "fnt")
            print(f"[{k}] ended={outcome['ended']} winner={outcome['winner']}: {states}")
            if k == 0:
                print("\n".join("    " + line for line in outcome["log"] if not line.startswith("|split")))
        return
    scenarios = select(args.scenario, args.family)
    if args.check:
        failed = 0
        for scenario in scenarios:
            try:
                replay = build(scenario)
            except ScenarioError as error:
                failed += 1
                print(f"FAIL {error}")
                continue
            print(f"== {scenario.name} ({scenario.family}, p{scenario.side + 1} decides): OK")
            print(describe_position(replay, scenario.side))
        if failed:
            raise SystemExit(f"{failed} scenario(s) do not reach their target")
        return
    if not args.config:
        raise SystemExit("give at least one --config")
    configs = [load_config(path) for path in args.config]
    names = [c["name"] for c in configs]
    if len(set(names)) != len(names):
        raise SystemExit(f"config names must differ: {names}")
    bots = [make_bot(c, args.checkpoint, args.set_pool) for c in configs]
    report = []
    for scenario in scenarios:
        entry = dict(name=scenario.name, family=scenario.family, side=scenario.side,
                     expect=None if scenario.expect is None else scenario.expect.describe(),
                     rationale=scenario.rationale, configs={})
        for config, bot in zip(configs, bots):
            runs = [decide_once(scenario, bot, derive_seed(args.seed, scenario.key(), r), config)
                    for r in range(args.repeats)]
            entry["configs"][config["name"]] = dict(summary=summarize(runs, scenario.expect), runs=runs)
        report.append(entry)
        print(table([entry], names).split("\n", 1)[1], flush=True)
    print()
    print(table(report, names))
    totals = {}
    for name in names:
        hard = [e for e in report if e["expect"] is not None]
        rates = [e["configs"][name]["summary"]["pass_rate"] for e in hard]
        passed = sum(r == 1.0 for r in rates)
        totals[name] = dict(scenarios=len(hard), all_pass=passed, mean_pass_rate=round(float(np.mean(rates)), 3)
                            if rates else None)
        print(f"{name}: {passed}/{len(hard)} scenarios pass in every repeat; mean pass rate "
              f"{totals[name]['mean_pass_rate']}")
    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(dict(args=vars(args), configs=configs, totals=totals, scenarios=report), indent=1,
                                  default=str))


if __name__ == "__main__":
    main()
