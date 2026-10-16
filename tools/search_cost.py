"""Measure serving-stage costs on a few protocol-fed positions, without changing a checkpoint.

Run from crates/pyengine with PYTHONPATH set to that directory, at nice -n 10::

    python ../../tools/search_cost.py --checkpoint /path/model.pt --pool /path/pool.txt.gz

Uses 2/8/32 worlds so the suggested nonnegative fixed+unit fits see different batch
sizes. The first decision warms model kernels and is excluded. CPU is the default,
matching ladder-mac.json. --depth2 also reports the core controller's stage models
when that branch is present. Output is JSON suitable for saving as a calibration
record; suggested_priors can be copied into bot.budget.priors.
"""
from __future__ import annotations

import argparse
import gzip
import json
import time
from pathlib import Path

import numpy as np

from training import search
from training.search2_types import Stage


def fit(samples):
    x = np.array([[1, s.units] for s in samples], dtype=float)
    y = np.array([s.ms for s in samples])
    if np.ptp(x[:, 1]) == 0:
        return {"fixed_ms": round(float(y.mean()), 5), "unit_ms": 0.0}
    fixed, unit = np.linalg.lstsq(x, y, rcond=None)[0]
    if fixed < 0:
        fixed, unit = 0.0, float(x[:, 1] @ y / (x[:, 1] @ x[:, 1]))
    if unit < 0:
        fixed, unit = float(y.mean()), 0.0
    return {"fixed_ms": round(float(fixed), 5), "unit_ms": round(float(unit), 6)}


def calibrate(args):
    import numbrion
    if args.depth2 and "depth2" not in search.SearchConfig.__dataclass_fields__:
        raise SystemExit("--depth2 needs the core search2 branch; layer-1 calibration works without it")
    settings = dict(ours=12, theirs=8, seeds=2, max_worlds=32, threads=args.threads)
    if args.depth2:
        settings["depth2"] = {"enabled": True}
    bot = search.SearchBot(checkpoint=args.checkpoint, pools=[args.pool], search=settings,
                           device=args.device, torch_threads=args.threads,
                           **({"budget": {"mode": "fixed"}} if args.depth2 else {}))
    bot.warmup()
    with gzip.open(args.pool, "rt") as stream:
        teams = [next(stream).strip() for _ in range(8)]
    battle = numbrion.Battle((11, 23, 5, 8), p1=teams[0], p2=teams[1], log=True)
    battle.start()
    player = bot.new_player("cost-calibration")
    original_candidates, original_values = bot.net.candidates, bot.net.values
    root_samples, value_samples, opp_units, in_search = [], [], [], [False]
    original_search = search.run_search

    def candidates(*a, **kw):
        start = time.perf_counter()
        result = original_candidates(*a, **kw)
        if not in_search[0] and not root_samples:
            root_samples.append(Stage("root_policy", 1, (time.perf_counter() - start) * 1000))
        elif in_search[0]:
            opp_units.append(len(a[1]))
        return result

    def values(*a, **kw):
        if not in_search[0]:
            return original_values(*a, **kw)
        arrays, partner_ids, partner_floats = a[:3]
        results = []
        chunk = kw.get("chunk", 2048)
        for start in range(0, len(arrays["field"]), chunk):
            rows = slice(start, start + chunk)
            begin = time.perf_counter()
            results.append(original_values({k: v[rows] for k, v in arrays.items()},
                                           partner_ids[rows], partner_floats[rows], chunk=chunk))
            value_samples.append(Stage("value1", len(results[-1]), (time.perf_counter() - begin) * 1000))
        return np.concatenate(results)

    def run_search(*a, **kw):
        in_search[0] = True
        try:
            return original_search(*a, **kw)
        finally:
            in_search[0] = False

    bot.net.candidates, bot.net.values, search.run_search = candidates, values, run_search
    records, samples = [], []
    try:
        attempts = 0
        while not battle.ended and len(records) <= args.positions and attempts < args.positions * 6 + 12:
            player.feed(battle.drain_player_log(0))
            battle.drain_player_log(1)
            if battle.needs_action(0):
                player.set_request(battle.request_json(0))
                move_turn = battle.request_kind(0) == battle.request_kind(1) == "move"
                count = args.worlds[len(records) % len(args.worlds)]
                bot.search.worlds = count
                root_samples.clear()
                value_samples.clear()
                opp_units.clear()
                attempts += 1
                start = time.perf_counter()
                decision = player.decide()
                elapsed = (time.perf_counter() - start) * 1000
                analysis = decision.analysis
                if analysis.get("source") == "search" and move_turn:
                    timing = analysis["timing"]
                    stages = root_samples + [Stage("worlds", analysis["worlds"], timing["worlds_ms"]),
                        Stage("opp_policy", sum(opp_units), timing["opponent_ms"]),
                        Stage("grid1", analysis["cells"], timing["grid_ms"])] + value_samples
                    records.append({"turn": battle.turn, "worlds": count, "total_ms": round(elapsed, 2),
                                    "stages": [vars(s) for s in stages]})
                    if len(records) > 1:
                        samples.extend(stages)
                battle.choose(0, decision.choice)
                player.record_choice(decision.choice)
            if battle.needs_action(1):
                battle.choose(1, "default")
    finally:
        bot.net.candidates, bot.net.values, search.run_search = original_candidates, original_values, original_search
    grouped = {name: [s for s in samples if s.name == name and s.units] for name in
               ("worlds", "root_policy", "opp_policy", "grid1", "value1")}
    if not records or any(not rows for rows in grouped.values()):
        raise RuntimeError("not enough searched positions to calibrate all stages")
    result = {"checkpoint": str(Path(args.checkpoint).resolve()), "width": bot.net.width, "layers": bot.net.layers,
              "device": args.device, "threads": args.threads, "warmup_excluded": records[0],
              "positions": records[1:], "suggested_priors": {name: fit(rows) for name, rows in grouped.items()},
              "stage_mean_ms": {name: round(float(np.mean([s.ms for s in rows])), 2)
                                for name, rows in grouped.items()},
              "stage_decision_mean_ms": {name: round(float(np.mean([
                  sum(s["ms"] for s in record["stages"] if s["name"] == name)
                  for record in records[1:]])), 2) for name in grouped},
              "mean_total_ms": round(float(np.mean([r["total_ms"] for r in records[1:]])), 2)}
    controller = getattr(bot, "budget", None)
    if args.depth2 and hasattr(controller, "stats"):
        result["depth2_cost_models"] = controller.stats()["stages"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", required=True)
    parser.add_argument("--pool", required=True)
    parser.add_argument("--positions", type=int, default=6)
    parser.add_argument("--worlds", nargs="+", type=int, default=[2, 8, 32])
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--depth2", action="store_true")
    args = parser.parse_args()
    if args.positions < 2 or any(n < 1 for n in args.worlds):
        parser.error("need at least two positions and positive world counts")
    print(json.dumps(calibrate(args), indent=2))


if __name__ == "__main__":
    main()
