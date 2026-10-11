"""Keep/drop decision for an overnight A/B test: a candidate checkpoint against the current best.

    .venv/bin/python crates/pyengine/compare.py --candidate RUN/snapshots/m060.pt --best BEST.pt --out RUN/compare

Matchups, each at every ``--temperatures`` value (it applies to every checkpoint; smart and heuristic are
unchanged), on mirrored battles of a held-out pool (evaluate.play):

* h2h: candidate vs best, ``--h2h-rounds`` x 2 x ``--envs`` games,
* panel: candidate vs smart, vs heuristic and vs the reference checkpoint ``--ref``, ``--panel-rounds`` each,
* the best checkpoint's own panel, for the panel-drop rule.

Each matchup type has a fixed battle seed and each matchup restarts the action RNG at ``ACTION_SEED``, so every
checkpoint faces the same teams and battle seeds. Results are cached in ``--cache`` by checkpoint, opponent,
temperature, seed, rounds, pool sha256, envs, workers, guards and device, so a checkpoint's panel is played once.

Decision at ``--decision-temperature``: KEEP if the h2h score is at least ``--bar`` and no panel score is more than
``--panel-drop`` below the best's (same opponent and temperature), else DROP. Without ``--best`` (the first control
run) only the panel is played. Writes OUT/compare.json and OUT/compare.md.

``--temperature-sweep CHECKPOINT`` instead plays one checkpoint at temperatures 1, 0.7, 0.5, 0.3 and 0 against
smart and against itself at temperature 1, and writes OUT/temperature-sweep.json.

Before every matchup that is not cached, the tool stops (exit code 3, partial results written) if ``--run-flag``
exists: a timed training run is going. Rerunning resumes from the cache.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import subprocess
import sys
import time
from dataclasses import asdict
from pathlib import Path

import numpy as np
import torch

from evaluate import PolicyAgent, load_policy, play
from training.damage import N_ACTION_FEATURES, N_TOKEN_FEATURES
from training.device import BACKENDS, training_device
from training.dex import load_dex
from training.encoder import N_POKEMON_FLOATS
from training.guards import MoveGuards
from training.scripted import HeuristicAgent
from training.smart import SmartHeuristicAgent

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
# Worktrees link data/teams to the main training checkout; the reference run and overnight outputs live there too.
SHARED = (REPO / "data/teams").resolve().parent.parent
DEFAULT_REF = SHARED / "scratch/training/w192-l4-rust-b996acb/snapshots/m180.pt"
DEFAULT_CACHE = SHARED / "scratch/training/overnight/panel-cache.json"
SEEDS = dict(h2h=900, smart=950, heuristic=952, ref=953, self=954)
ACTION_SEED = 77
PANEL = ("smart", "heuristic", "ref")
SWEEP_TEMPERATURES = [1.0, 0.7, 0.5, 0.3, 0.0]
CACHED_KEYS = ("games", "score", "wins", "ties", "pair_sweeps", "pair_splits", "pair_losses", "steps")


class RunActive(Exception):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_state() -> dict:
    """Commit and whether tracked files differ from it (worktrees carry an untracked data/teams link)."""
    def git(*command):
        return subprocess.run(["git", "-C", str(REPO), *command], capture_output=True, text=True).stdout.strip()
    try:
        return dict(commit=git("rev-parse", "HEAD"), branch=git("rev-parse", "--abbrev-ref", "HEAD"),
                    dirty=bool(git("status", "--porcelain", "--untracked-files=no")))
    except OSError:
        return dict(commit=None, branch=None, dirty=None)


def describe(path: Path) -> dict:
    """sha256, shape and feature column counts of a checkpoint (as Model.from_state infers them)."""
    state = torch.load(path, map_location="cpu", weights_only=False)
    weights = state["model"]
    info = dict(path=str(path), sha256=sha256(path), width=state.get("width", 128), layers=state.get("layers", 3),
                action_features=int(weights["act_damage.weight"].shape[1]),
                token_features=int(weights["float_proj.weight"].shape[1]) - N_POKEMON_FLOATS,
                **{k: round(state[k], 2) for k in ("minutes", "battles", "update") if k in state})
    if info["action_features"] > N_ACTION_FEATURES or info["token_features"] > N_TOKEN_FEATURES:
        raise SystemExit(f"{path} reads {info['action_features']} action / {info['token_features']} token feature "
                         f"columns but this code's encoder makes {N_ACTION_FEATURES} / {N_TOKEN_FEATURES}: run "
                         f"compare.py from a checkout with the newer encoder")
    return info


def cache_key(checkpoint: str, opponent: str, temperature: float, seed: int, rounds: int, pool: str, envs: int,
              workers: int, guards: MoveGuards, device: str, action_seed: int = ACTION_SEED) -> str:
    """Everything that decides a matchup's games: ``checkpoint``/``pool`` are sha256s, ``opponent`` is smart,
    heuristic or ``policy:<sha256>@<temperature>``. The engine and scripted players' code is not part of it."""
    fields = dict(checkpoint=checkpoint, opponent=opponent, temperature=float(temperature), seed=seed,
                  rounds=rounds, pool=pool, envs=envs, workers=workers, action_seed=action_seed,
                  guards=",".join(f"{k}={v}" for k, v in asdict(guards).items()), device=device)
    return "|".join(f"{k}={v}" for k, v in fields.items())


def pair_interval(result: dict) -> list:
    """95% interval of a mirrored matchup's score with the pairs as samples, from the pair counts of evaluate.play.
    Pairs with one tie (1.5 or 0.5 points) are not counted there; the score's total tells how many have 1.5."""
    n = result["games"] // 2
    p = result["score"]
    unclassified = n - sum(result[k] for k in ("pair_sweeps", "pair_splits", "pair_losses"))
    remainder = n * p - result["pair_sweeps"] - 0.5 * result["pair_splits"]
    high = round((remainder - 0.25 * unclassified) / 0.5)
    assert 0 <= high <= unclassified
    low = unclassified - high
    second = (result["pair_sweeps"] + 0.25 * result["pair_splits"] + 0.5625 * high + 0.0625 * low) / n
    error = 1.96 * math.sqrt(max(0.0, (second - p * p) / max(n - 1, 1)))
    if p in (0, 1):
        bound = 1.96 ** 2 / (n + 1.96 ** 2)
        return [0.0, bound] if p == 0 else [1 - bound, 1.0]
    return [max(0.0, p - error), min(1.0, p + error)]


def difference_interval(a: dict, b: dict) -> list | None:
    """95% interval of a's minus b's score over the mirrored pairs both played (same teams and battle seeds)."""
    def means(result):
        p = result["pairs"]
        return {(e, k): (x + y) / 2 for e, k, x, y in zip(p["env"], p["battle"], p["first"], p["mirrored"])}

    if "pairs" not in a or "pairs" not in b:
        return None
    ours, theirs = means(a), means(b)
    d = np.array([ours[key] - theirs[key] for key in sorted(ours.keys() & theirs.keys())])
    if len(d) < 2:
        return None
    error = 1.96 * d.std(ddof=1) / math.sqrt(len(d))
    return [float(d.mean() - error), float(d.mean() + error)]


def decide(h2h: dict, panel: dict, best_panel: dict, bar: float, panel_drop: float) -> dict:
    """KEEP if the h2h score is at least ``bar`` and no panel score (``panel``, ``best_panel``: opponent -> result at
    one temperature) is more than ``panel_drop`` below the best's, else DROP."""
    eps = 1e-9  # scores are ratios of game counts: 0.5 - 0.53 must not count as a drop of more than 0.03
    rows, reasons = [], []
    h2h_ok = h2h["score"] >= bar - eps
    if not h2h_ok:
        reasons.append(f"h2h {h2h['score']:.4f} < bar {bar}")
    for opponent, result in panel.items():
        best = best_panel[opponent]
        difference = result["score"] - best["score"]
        ok = difference >= -panel_drop - eps
        rows.append(dict(opponent=opponent, candidate=result["score"], best=best["score"], difference=difference,
                         difference_95pct_interval=difference_interval(result, best), ok=ok))
        if not ok:
            reasons.append(f"vs {opponent} {difference:+.4f} below best by more than {panel_drop}")
    return dict(verdict="DROP" if reasons else "KEEP", h2h=h2h["score"], h2h_95pct_interval=pair_interval(h2h),
                h2h_ok=h2h_ok, bar=bar, panel_drop=panel_drop, panel=rows, reasons=reasons)


def dumps(data) -> str:
    """Indented JSON with lists of numbers on one line (per-pair outcomes)."""
    return re.sub(r"\[\s+([-+.\deE,\s]+?)\s*\]", lambda m: "[" + " ".join(m.group(1).split()) + "]",
                  json.dumps(data, indent=1))


def load_cache(path: Path) -> dict:
    return json.loads(path.read_text())["entries"] if path.exists() else {}


class Evaluator:
    """Matchups of checkpoints by role (candidate, best, ref, ...) against smart, heuristic or each other."""

    def __init__(self, args, checkpoints: dict):
        torch.set_num_threads(2)
        self.args = args
        self.dex = load_dex()
        self.device, self.device_name = training_device(args.device)
        self.guards = MoveGuards()
        self.pool_sha = sha256(args.pool)
        self.git = git_state()
        self.checkpoints = {role: describe(path) for role, path in checkpoints.items()}
        self.models = {}
        self.cache = load_cache(args.cache)

    def policy(self, role: str, temperature: float, rng) -> PolicyAgent:
        if role not in self.models:
            self.models[role] = load_policy(Path(self.checkpoints[role]["path"]), self.dex, self.device, rng,
                                            name=role).model
        return PolicyAgent(self.models[role], self.device, rng, role, temperature=temperature, guards=self.guards)

    def matchup(self, role: str, opponent: str, temperature: float, seed: int, rounds: int,
                opponent_temperature: float | None = None) -> dict:
        """``role``'s checkpoint at ``temperature`` against smart, heuristic or a role (at ``opponent_temperature``,
        default ``temperature``); cached results are reused."""
        if opponent in ("smart", "heuristic"):
            opponent_temperature, opponent_id = None, opponent
        else:
            opponent_temperature = float(temperature if opponent_temperature is None else opponent_temperature)
            opponent_id = f"policy:{self.checkpoints[opponent]['sha256']}@{opponent_temperature}"
        args = self.args
        key = cache_key(self.checkpoints[role]["sha256"], opponent_id, temperature, seed, rounds, self.pool_sha,
                        args.envs, args.workers, self.guards, str(self.device))
        entry = self.cache.get(key)
        cached = entry is not None
        if not cached:
            if args.run_flag and Path(args.run_flag).exists():
                raise RunActive(f"{args.run_flag} exists: stopped before {role} vs {opponent} at T={temperature}")
            rng = np.random.default_rng(ACTION_SEED)
            a = self.policy(role, temperature, rng)
            if opponent == "smart":
                b = SmartHeuristicAgent(rng, self.dex)
            elif opponent == "heuristic":
                b = HeuristicAgent(rng)
            else:
                b = self.policy(opponent, opponent_temperature, rng)
            started = time.time()
            result, *_ = play(a, b, args.pool, args.envs, rounds, seed, args.workers, pairs=True)
            entry = {k: result[k] for k in CACHED_KEYS}
            entry.update(seconds=round(time.time() - started, 1), played=time.strftime("%Y-%m-%d %H:%M:%S"),
                         commit=self.git["commit"], dirty=self.git["dirty"], pairs=result["pairs"])
            self.cache = {**load_cache(args.cache), key: entry}  # another compare may have added entries
            args.cache.parent.mkdir(parents=True, exist_ok=True)
            temporary = args.cache.with_suffix(".tmp")
            temporary.write_text(dumps(dict(version=1, entries=self.cache)))
            temporary.replace(args.cache)
        if entry["games"] != 2 * args.envs * rounds:
            print(f"WARNING: {role} vs {opponent} finished {entry['games']} of {2 * args.envs * rounds} games")
        out = dict(a=role, b=opponent, temperature=float(temperature), b_temperature=opponent_temperature, seed=seed,
                   rounds=rounds, **{k: entry[k] for k in CACHED_KEYS}, pair_95pct_interval=pair_interval(entry),
                   cached=cached, seconds=entry["seconds"], played=entry["played"], commit=entry["commit"],
                   cache_key=key, pairs=entry["pairs"])
        low, high = out["pair_95pct_interval"]
        same = opponent_temperature in (None, float(temperature))
        against = opponent if same else f"{opponent} (T={opponent_temperature:g})"
        took = "cached" if cached else f"{entry['seconds']}s"
        print(f"{role} vs {against} at T={temperature:g}: {out['score']:.4f} [{low:.4f}, {high:.4f}] over "
              f"{out['games']} games ({took})", flush=True)
        return out

    def manifest(self, temperatures: list) -> dict:
        args = self.args
        return dict(command=sys.argv, git=self.git, checkpoints=self.checkpoints,
                    encoder=dict(action_features=N_ACTION_FEATURES, token_features=N_TOKEN_FEATURES),
                    pool=dict(path=str(args.pool), resolved=str(args.pool.resolve()), sha256=self.pool_sha),
                    temperatures=temperatures, seeds=SEEDS, action_seed=ACTION_SEED, guards=asdict(self.guards),
                    envs_per_copy=args.envs, workers=args.workers, h2h_rounds=args.h2h_rounds,
                    panel_rounds=args.panel_rounds, device=str(self.device), device_name=self.device_name,
                    torch=torch.__version__, cache=str(args.cache), started=time.strftime("%Y-%m-%d %H:%M:%S"),
                    finished=None)


def fmt(result: dict) -> str:
    low, high = result["pair_95pct_interval"]
    return f"{result['score']:.3f} [{low:.3f}, {high:.3f}]"


def summary(out: dict) -> list:
    """The decision lines printed to stdout and put at the top of compare.md."""
    lines = []
    if out["manifest"]["checkpoints"].get("best") is None:
        lines.append("Control run (no --best): panel only.")
        for row in out["matchups"]:
            lines.append(f"  T={row['temperature']:g} {row['a']} vs {row['b']}: {fmt(row)}")
        return lines
    if out["decision"] is None:
        lines.append("No decision: the decision temperature's matchups did not finish.")
    for decision in out["decisions"]:
        main = decision["temperature"] == out["decision_temperature"]
        low, high = decision["h2h_95pct_interval"]
        lines.append(f"{'DECISION' if main else 'info'} at T={decision['temperature']:g}: {decision['verdict']}"
                     f"{'' if main else ' (not the decision temperature)'}")
        lines.append(f"  h2h candidate vs best: {decision['h2h']:.4f} [{low:.4f}, {high:.4f}] "
                     f"({'>=' if decision['h2h_ok'] else '<'} bar {decision['bar']})")
        for row in decision["panel"]:
            interval = row["difference_95pct_interval"]
            paired = f", paired 95% [{interval[0]:+.4f}, {interval[1]:+.4f}]" if interval else ""
            verdict = "ok" if row["ok"] else "more than {} below best".format(decision["panel_drop"])
            lines.append(f"  vs {row['opponent']}: candidate {row['candidate']:.4f}, best {row['best']:.4f}, "
                         f"difference {row['difference']:+.4f}{paired} ({verdict})")
    return lines


def markdown(out: dict) -> str:
    m = out["manifest"]
    lines = [f"# Compare: {out['label']}", "", "```", *summary(out), "```", "",
             "| a | b | T | seed | games | score | pair 95% | wins | ties | sweeps / splits / losses | cached |",
             "|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in out["matchups"]:
        low, high = r["pair_95pct_interval"]
        b = r["b"] if r["b_temperature"] is None or r["b_temperature"] == r["temperature"] else \
            f"{r['b']} (T={r['b_temperature']:g})"
        lines.append(f"| {r['a']} | {b} | {r['temperature']:g} | {r['seed']} | {r['games']} | {r['score']:.4f} | "
                     f"[{low:.3f}, {high:.3f}] | {r['wins']} | {r['ties']} | {r['pair_sweeps']} / "
                     f"{r['pair_splits']} / {r['pair_losses']} | {'yes' if r['cached'] else 'no'} |")
    lines += ["", "## Manifest", ""]
    for role, c in m["checkpoints"].items():
        extra = "".join(f", {k} {c[k]}" for k in ("minutes", "battles", "update") if k in c)
        lines.append(f"- {role}: `{c['path']}` sha256 `{c['sha256'][:16]}`, {c['width']}x{c['layers']}, "
                     f"{c['action_features']} action / {c['token_features']} token features{extra}")
    git = m["git"]
    lines += [f"- encoder: {m['encoder']['action_features']} action / {m['encoder']['token_features']} token features",
              f"- git: {git['commit']} ({git['branch']}){' dirty' if git['dirty'] else ''}",
              f"- pool: `{m['pool']['path']}` sha256 `{m['pool']['sha256'][:16]}`",
              f"- temperatures {m['temperatures']}, seeds {m['seeds']}, action seed {m['action_seed']}, "
              f"guards {m['guards']}",
              f"- {m['envs_per_copy']} envs per mirrored copy, {m['workers']} workers, rounds h2h {m['h2h_rounds']} / "
              f"panel {m['panel_rounds']}, device {m['device']} ({m['device_name']})",
              f"- started {m['started']}, finished {m['finished']}"]
    if out.get("stopped"):
        lines.append(f"- STOPPED: {out['stopped']}")
    return "\n".join(lines) + "\n"


def compare(args) -> int:
    roles = dict(candidate=args.candidate, best=args.best, ref=args.ref)
    evaluator = Evaluator(args, {role: path for role, path in roles.items() if path is not None})
    temperatures = list(dict.fromkeys(float(t) for t in args.temperatures + [args.decision_temperature]))
    out = dict(label=args.label or args.out.name, mode="compare", decision_temperature=args.decision_temperature,
               manifest=evaluator.manifest(temperatures), decision=None, decisions=[], matchups=[])
    args.out.mkdir(parents=True, exist_ok=True)

    def write():
        (args.out / "compare.json").write_text(dumps(out))
        (args.out / "compare.md").write_text(markdown(out))

    try:
        for temperature in sorted(temperatures, key=lambda t: t != args.decision_temperature):
            panel, best_panel = {}, {}
            if args.best is not None:
                h2h = evaluator.matchup("candidate", "best", temperature, SEEDS["h2h"], args.h2h_rounds)
                out["matchups"].append(h2h)
                write()
            for opponent in PANEL:
                panel[opponent] = evaluator.matchup("candidate", opponent, temperature, SEEDS[opponent],
                                                    args.panel_rounds)
                out["matchups"].append(panel[opponent])
                write()
            if args.best is None:
                continue
            for opponent in PANEL:
                best_panel[opponent] = evaluator.matchup("best", opponent, temperature, SEEDS[opponent],
                                                         args.panel_rounds)
                out["matchups"].append(best_panel[opponent])
                write()
            decision = dict(temperature=temperature, **decide(h2h, panel, best_panel, args.bar, args.panel_drop))
            out["decisions"].append(decision)
            if temperature == args.decision_temperature:
                out["decision"] = decision
    except RunActive as error:
        out["stopped"] = str(error)
    out["manifest"]["finished"] = time.strftime("%Y-%m-%d %H:%M:%S")
    write()
    print("\n".join(summary(out)), flush=True)
    if out.get("stopped"):
        print(f"STOPPED: {out['stopped']}; rerun to continue from the cache", flush=True)
        return 3
    print(f"Wrote {args.out / 'compare.json'} and compare.md", flush=True)
    return 0


def sweep(args) -> int:
    evaluator = Evaluator(args, dict(checkpoint=args.temperature_sweep))
    temperatures = [float(t) for t in (args.temperatures or SWEEP_TEMPERATURES)]
    out = dict(label=args.label or args.out.name, mode="temperature-sweep", manifest=evaluator.manifest(temperatures),
               table=[], matchups=[])
    args.out.mkdir(parents=True, exist_ok=True)
    try:
        for temperature in temperatures:
            smart = evaluator.matchup("checkpoint", "smart", temperature, SEEDS["smart"], args.panel_rounds)
            out["matchups"].append(smart)
            own = evaluator.matchup("checkpoint", "checkpoint", temperature, SEEDS["self"], args.panel_rounds,
                                    opponent_temperature=1.0)
            out["matchups"].append(own)
            out["table"].append(dict(temperature=temperature, vs_smart=smart["score"],
                                     vs_smart_95pct=smart["pair_95pct_interval"], vs_self_t1=own["score"],
                                     vs_self_t1_95pct=own["pair_95pct_interval"], games_each=smart["games"]))
            (args.out / "temperature-sweep.json").write_text(dumps(out))
    except RunActive as error:
        out["stopped"] = str(error)
    out["manifest"]["finished"] = time.strftime("%Y-%m-%d %H:%M:%S")
    (args.out / "temperature-sweep.json").write_text(dumps(out))
    print(f"\n{'T':>4}  {'vs smart':>8}  {'95% pairs':>16}  {'vs self T=1':>11}  {'95% pairs':>16}  games")
    for row in out["table"]:
        print(f"{row['temperature']:>4g}  {row['vs_smart']:>8.4f}  [{row['vs_smart_95pct'][0]:.3f}, "
              f"{row['vs_smart_95pct'][1]:.3f}]  {row['vs_self_t1']:>11.4f}  [{row['vs_self_t1_95pct'][0]:.3f}, "
              f"{row['vs_self_t1_95pct'][1]:.3f}]  {row['games_each']}", flush=True)
    if out.get("stopped"):
        print(f"STOPPED: {out['stopped']}; rerun to continue from the cache", flush=True)
        return 3
    print(f"Wrote {args.out / 'temperature-sweep.json'}", flush=True)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--best", type=Path, help="omit for the first control run (panel only)")
    parser.add_argument("--temperature-sweep", type=Path, metavar="CHECKPOINT",
                        help="play this checkpoint at several temperatures vs smart and vs itself at T=1")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--label")
    parser.add_argument("--temperatures", type=float, nargs="+",
                        help="default 1.0 0.5 (the decision temperature is always added); sweep: 1 0.7 0.5 0.3 0")
    parser.add_argument("--decision-temperature", type=float, default=0.5)
    parser.add_argument("--h2h-rounds", type=int, default=40, help="battles per env: games = 2 * envs * rounds")
    parser.add_argument("--panel-rounds", type=int, default=20, help="also the sweep's rounds per matchup")
    parser.add_argument("--pool", type=Path, default=REPO / "data/teams/eval-s43-200.txt")
    parser.add_argument("--ref", type=Path, default=DEFAULT_REF)
    parser.add_argument("--bar", type=float, default=0.53, help="minimum h2h score to keep the candidate")
    parser.add_argument("--panel-drop", type=float, default=0.03,
                        help="largest allowed drop of a panel score below the best's")
    parser.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--run-flag", default="/tmp/numbrion-overnight/RUN_ACTIVE",
                        help="stop before an uncached matchup while this file exists ('' to ignore)")
    parser.add_argument("--envs", type=int, default=50, help="battles per mirrored copy at a time")
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--device", choices=BACKENDS, default="auto")
    args = parser.parse_args()
    if (args.candidate is None) == (args.temperature_sweep is None):
        parser.error("give exactly one of --candidate and --temperature-sweep")
    if args.temperature_sweep is None:
        args.temperatures = args.temperatures or [1.0, 0.5]
    sys.exit(compare(args) if args.candidate is not None else sweep(args))


if __name__ == "__main__":
    main()
