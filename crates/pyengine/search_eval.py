"""Does search beat the plain policy? Mirrored games between a SearchBot and a policy (or the smart player).

    .venv/bin/python crates/pyengine/search_eval.py --checkpoint CKPT --pairs 200 --out scratch/search-eval/run.json

Every pair is two battles with the same teams and battle seed, the search bot on side 1 in the first and side 2 in
the second, so team and seed luck cancel (as evaluate.py's duplicate games). The search bot is protocol-driven
(training.player): it sees only its player-view lines and requests and builds its own worlds. The opponent is the
base policy as evaluate.py/compare.py play it: temperature ``--base-temperature`` (0.5), MoveGuards, the engine's
legal masks. ``--opponent smart`` plays Rust's smart scripted player instead.

Score = the search bot's mean result (win 1, tie 0.5); the error is the standard error of the pair means. Games run
``--concurrency`` at a time in threads (the engine, world builder and encoder release the GIL).

Two search bots: ``--opponent search`` with ``--opponent-search``; both configs may carry a ``depth2`` block (two-ply
search) and can be JSON files. Each protocol player gets its own seed from (``--seed``, pair, game, battle side), so
with bots that take per-player seeds (``SearchBot.new_player(room, seed=...)``) and fixed sizes (no ms budgets) a
game's searches do not depend on concurrency.

Reported besides the score (``summary`` in ``--out``, and the progress lines):

* capped games (stopped at ``--max-turns``, often stalling): their count, and a second score that counts them as
  ties instead of leaving their pairs out;
* per-decision wall time per side (``timing``: mean, p50, p90, p99, max; all decisions and searched ones);
* the second layer per side when decisions carry a ``depth2`` analysis block: how often it ran, changed the
  choice, or was skipped (by reason), and its mean time.

``--positions-out`` keeps a sample of decision points (packed teams, battle seed, the choices so far, the side to
move) as JSONL, replayable with tools/positions.py (tools/search_timing.py replays them for timing).
"""
from __future__ import annotations

import argparse
import json
import math
import os
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import torch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
REPO = HERE.parents[1]

import numbrion  # noqa: E402

from training.encoder import Buffers  # noqa: E402
from training.guards import MoveGuards  # noqa: E402
from training.search import N_ACTIONS, OBS_KEYS, SearchBot  # noqa: E402
from tools.positions import derive_seed, new_player  # noqa: E402

RUN_FLAG = Path("/tmp/numbrion-overnight/RUN_ACTIVE")


def gumbel_pick(logits: np.ndarray, mask: np.ndarray, temperature: float, rng) -> int:
    if temperature <= 0:
        return int(np.argmax(np.where(mask, logits, -np.inf)))
    z = np.where(mask, logits / temperature, -np.inf) + rng.gumbel(size=logits.shape)
    return int(np.argmax(z))


class BasePolicy:
    """evaluate.PolicyAgent for one side of one battle: engine masks, guards, per-slot tempered sampling."""

    def __init__(self, net, dex_path: str, side: int, temperature: float, rng):
        self.net, self.side, self.temperature, self.rng = net, side, temperature, rng
        self.encoder = numbrion.ObsEncoder(dex_path, 2)
        self.buffers = Buffers(2)
        self.pending: list[str] = []
        self.guards = MoveGuards()

    def feed(self, lines):
        self.pending.extend(lines)

    def act(self, battle) -> tuple[int, int]:
        b = self.buffers
        outs = (b.ids, b.floats, b.field, b.foe_match, b.fainted, b.guard, b.action_features, b.token_features)
        self.encoder.encode_rows([self.side], [dict(battle_id=1, request=battle.request_json(self.side),
                                                    log=self.pending)], *outs)
        self.pending = []
        mask0, _ = battle.legal_mask(self.side)
        guard = b.guard[[self.side]]
        mask0, _ = self.guards.apply(mask0[None], guard, 0)
        net = self.net
        model = net.policy_model  # CPU copy: one row per call
        with net.policy_lock, torch.no_grad():
            x = {k: torch.from_numpy(np.ascontiguousarray(getattr(b, k)[[self.side]])) for k in OBS_KEYS}
            state = model.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
            lp0, vectors0 = model.slot0(state, torch.from_numpy(mask0))
            a0 = gumbel_pick(lp0.numpy()[0], mask0[0], self.temperature, self.rng)
            mask1 = battle.legal_mask_slot1(self.side, a0)[None]
            mask1, _ = self.guards.apply(mask1, guard, 1)
            lp1 = model.slot1(state, vectors0, torch.tensor([a0]), torch.from_numpy(mask1))
            a1 = gumbel_pick(lp1.numpy()[0], mask1[0], self.temperature, self.rng)
        return a0, a1


class SmartOpponent:
    """Rust's smart scripted player (training.smart) for one side."""

    def __init__(self, dex_path: str, side: int, seed: int):
        self.side = side
        self.encoder = numbrion.ObsEncoder(dex_path, 2)
        self.buffers = Buffers(2)
        self.agent = numbrion.SmartAgent(dex_path, seed, temperature=0.05)
        self.pending: list[str] = []

    def feed(self, lines):
        self.pending.extend(lines)

    def act(self, battle) -> tuple[int, int]:
        b = self.buffers
        outs = (b.ids, b.floats, b.field, b.foe_match, b.fainted, b.guard, b.action_features, b.token_features)
        self.encoder.encode_rows([self.side], [dict(battle_id=1, request=battle.request_json(self.side),
                                                    log=self.pending)], *outs)
        self.pending = []
        mask0, _ = battle.legal_mask(self.side)
        rows = np.array([self.side], dtype=np.int64)
        a0 = int(self.agent.first(b.ids, b.floats, b.field, b.action_features, b.token_features, rows,
                                  mask0[None].copy())[0])
        mask1 = battle.legal_mask_slot1(self.side, a0)[None].copy()
        a1 = int(self.agent.second(mask1)[0])
        return a0, a1


class SearchOpponent:
    """Another SearchBot (e.g. a different budget or search config) as the opponent, driven like the evaluated bot."""

    def __init__(self, bot: SearchBot, side: int, team: str, engine_mask: bool, budget_ms: float | None = None,
                 seed: int | None = None):
        self.side, self.engine_mask = side, engine_mask
        self.budget_ms = budget_ms
        self.worlds: list[int] = []
        self.player, _ = new_player(bot, "opponent", seed)
        self.player.team = team

    def feed(self, lines):
        self.player.feed(lines)

    def choose(self, battle, record: list | None = None) -> int:
        return choose_with_retries(self.player, battle, self.side, self.engine_mask, self.budget_ms, self.worlds,
                                   record)


def depth2_info(analysis: dict) -> dict | None:
    """The second layer's outcome for one decision, from its analysis ``depth2`` block (training/search2.py:
    ``rounds`` completed, ``chosen``, ``changed``, ``skipped`` = why it stopped or did not run, ``total_ms``), or None
    when the decision has none. ``ran``: at least one round finished and its choice was used; ``reason``: why it did
    not run; ``partial``: why it stopped early after a finished round; ``ms``: its time."""
    block = analysis.get("depth2") if isinstance(analysis, dict) else None
    if not isinstance(block, dict):
        return None
    skipped = block.get("skipped")
    if "rounds" in block:
        ran = bool(block["rounds"]) and block.get("chosen") is not None
    else:  # an older or reduced block: explicit flags
        ran = bool(block.get("ran", not skipped))
    timing = block.get("timing") if isinstance(block.get("timing"), dict) else {}
    ms = block.get("total_ms", block.get("time_ms"))
    if ms is None and timing:
        ms = sum(v for v in timing.values() if isinstance(v, (int, float)))
    reason = None if ran else str(skipped or block.get("reason") or "skipped")
    partial = str(skipped) if ran and skipped else None
    return dict(ran=ran, changed=ran and bool(block.get("changed", False)), reason=reason, partial=partial,
                ms=None if ms is None else float(ms))


def choose_with_retries(player, battle, side: int, engine_mask: bool, budget_ms: float | None, worlds: list,
                        record: list | None = None) -> int:
    """A protocol-driven player's choice for ``side``. A rejected choice (e.g. a switch out of an unrevealed Shadow Tag
    trap: the request said only maybeTrapped) is fed back with the engine's updated request and retried; after three
    rejections the engine's default choice is made. Returns the number of rejections.

    ``record`` (optional) gets one dict for the decision: wall time ``ms`` over all attempts, the final attempt's
    ``source`` and ``depth2`` outcome, the accepted codes (``a0``/``a1``, None for the default choice) and the
    rejected code pairs."""
    started = time.perf_counter()
    player.set_request(battle.request_json(side))
    rejected = 0
    rejected_codes: list = []
    entry = dict(source=None, depth2=None, choice=None, a0=None, a1=None, worlds=None)
    accepted = False
    for attempt in range(3):
        if engine_mask:
            player.mask_hint = battle.legal_mask(side)
        player.budget_ms = budget_ms
        decision = player.decide()
        if decision is None:
            break
        analysis = decision.analysis or {}
        entry.update(source=analysis.get("source"), depth2=depth2_info(analysis),
                     choice=getattr(decision, "choice", None), worlds=analysis.get("worlds"),
                     value=analysis.get("value"))
        if analysis.get("source") == "search":
            worlds.append(analysis["worlds"])
        try:
            battle.choose_action(side, decision.a0, decision.a1)
            entry.update(a0=int(decision.a0), a1=int(decision.a1))
            accepted = True
            break
        except numbrion.ChoiceError as error:
            rejected += 1
            rejected_codes.append([int(decision.a0), int(decision.a1)])
            player.rejected(str(error.args[0]))
            if len(error.args) > 1 and error.args[1]:
                player.set_request(error.args[1])
    if not accepted:
        battle.choose(side, "default")
        entry.update(choice="default")
    if record is not None:
        record.append(dict(entry, ms=(time.perf_counter() - started) * 1000, rejected=rejected_codes))
    return rejected


def play_game(bot: SearchBot, opponent_factory, team1: str, team2: str, seed: tuple, search_side: int,
              engine_mask: bool, max_turns: int = 400, budget_ms: float | None = None, player_seed: int | None = None,
              opponent_seed: int | None = None, positions_rate: float = 0.0, positions_rng=None) -> dict:
    """One battle, the search bot on ``search_side``. ``player_seed``/``opponent_seed`` seed the protocol players'
    own random numbers (bots that support it), so a game does not depend on what other games run concurrently.
    With ``positions_rate`` > 0 each decision point is kept with that probability (``positions_rng``) as a
    replayable position (tools/positions.py) under the result's ``positions``."""
    battle = numbrion.Battle(seed, p1=team1, p2=team2, log=True)
    battle.start()
    player, _ = new_player(bot, f"eval-{seed}", player_seed)
    player.team = team1 if search_side == 0 else team2
    opponent_team = team2 if search_side == 0 else team1
    if opponent_seed is None:
        opponent = opponent_factory(1 - search_side, opponent_team)
    else:
        opponent = opponent_factory(1 - search_side, opponent_team, seed=opponent_seed)
    rejected = opponent_rejected = 0
    worlds: list[int] = []
    history: list[dict] = []
    positions: list[dict] = []
    records = {"search": [], "opponent": []}
    while not battle.ended and battle.turn <= max_turns:
        lines = [battle.drain_player_log(s) for s in (0, 1)]
        player.feed(lines[search_side])
        opponent.feed(lines[1 - search_side])
        order = [s for s in (0, 1) if battle.needs_action(s)]
        for side in order:
            role = "search" if side == search_side else "opponent"
            position = None
            if positions_rate > 0 and positions_rng is not None and positions_rng.random() < positions_rate:
                position = dict(teams=[team1, team2], seed=[int(x) for x in seed], names=None, side=side,
                                turn=int(battle.turn), role=role, request_kind=battle.request_kind(side),
                                history=[dict(step) for step in history])
            record = records[role]
            if side == search_side:
                rejected += choose_with_retries(player, battle, side, engine_mask, budget_ms, worlds, record)
            elif isinstance(opponent, SearchOpponent):
                opponent_rejected += opponent.choose(battle, record)
            else:
                started = time.perf_counter()
                a0, a1 = opponent.act(battle)
                battle.choose_action(side, a0, a1)
                record.append(dict(source="policy", depth2=None, a0=int(a0), a1=int(a1), rejected=[],
                                   ms=(time.perf_counter() - started) * 1000))
            last = record[-1]
            step = dict(side=side, a0=last["a0"], a1=last["a1"])
            if last["a0"] is None:
                step["choice"] = "default"
            if last["rejected"]:
                step["rejected"] = last["rejected"]
            history.append(step)
            if position is not None:
                position["decision"] = {k: last.get(k) for k in ("choice", "source", "worlds", "value")}
                position["decision"]["ms"] = round(last["ms"], 1)
                positions.append(position)
    # A battle stopped at the turn cap has no result (None): it is reported as capped (see summarize).
    score = None if not battle.ended else 0.5 if battle.tie or battle.winner is None else float(battle.winner == search_side)
    opponent_worlds = opponent.worlds if isinstance(opponent, SearchOpponent) else []
    result = dict(score=score, turns=battle.turn, ended=battle.ended, capped=not battle.ended, rejected=rejected,
                  opponent_rejected=opponent_rejected, stats=dict(player.stats),
                  search_side=search_side, worlds_count=len(worlds),
                  worlds_mean=float(np.mean(worlds)) if worlds else None,
                  opponent_worlds_count=len(opponent_worlds),
                  opponent_worlds_mean=float(np.mean(opponent_worlds)) if opponent_worlds else None,
                  decisions={role: decision_summary(entries) for role, entries in records.items()})
    if positions:
        result["positions"] = positions
    return result


def decision_summary(entries: list) -> dict:
    """Per game and role: decision times (ms, rounded) with their sources, and depth2 outcomes."""
    return dict(ms=[round(e["ms"], 1) for e in entries], source=[e.get("source") or "default" for e in entries],
                depth2=[e["depth2"] for e in entries if e.get("depth2") is not None])


def worlds_means(results: list) -> dict:
    """Decision-weighted means across games; policy/forced decisions do not contribute."""
    means = {}
    for prefix in ("", "opponent_"):
        count = sum(g[f"{prefix}worlds_count"] for r in results for g in r["games"])
        total = sum((g[f"{prefix}worlds_mean"] or 0) * g[f"{prefix}worlds_count"]
                    for r in results for g in r["games"])
        means[f"{prefix}worlds_mean"] = total / count if count else None
    return means


def time_stats(values) -> dict | None:
    """count, mean, p50, p90, p99, max of decision times in ms (None without decisions)."""
    values = np.asarray(list(values), dtype=float)
    if not len(values):
        return None
    p50, p90, p99 = np.percentile(values, [50, 90, 99])
    return dict(count=int(len(values)), mean=round(float(values.mean()), 1), p50=round(float(p50), 1),
                p90=round(float(p90), 1), p99=round(float(p99), 1), max=round(float(values.max()), 1))


def timing_summary(results: list) -> dict:
    """Per role ("search" = the evaluated bot, "opponent"): time stats of every decision and of searched ones."""
    out = {}
    for role in ("search", "opponent"):
        ms, searched = [], []
        for r in results:
            for g in r["games"]:
                d = (g.get("decisions") or {}).get(role)
                if not d:
                    continue
                ms += d["ms"]
                searched += [m for m, s in zip(d["ms"], d["source"]) if s == "search"]
        out[role] = dict(all=time_stats(ms), search=time_stats(searched))
    return out


def depth2_summary(results: list) -> dict | None:
    """Per role: how often the second layer ran, changed the choice, was skipped (by reason) or stopped after a
    finished round (``stopped_early``, by reason); its time. Shares are of the role's searched decisions. None when no
    decision had a depth2 block."""
    out = {}
    for role in ("search", "opponent"):
        searched = blocks = ran = changed = 0
        reasons: dict = {}
        partial: dict = {}
        ms = []
        for r in results:
            for g in r["games"]:
                d = (g.get("decisions") or {}).get(role)
                if not d:
                    continue
                searched += sum(s == "search" for s in d["source"])
                for info in d["depth2"]:
                    blocks += 1
                    if info["ran"]:
                        ran += 1
                        changed += bool(info["changed"])
                        if info.get("ms") is not None:
                            ms.append(info["ms"])
                        if info.get("partial"):
                            key = str(info["partial"])[:80]
                            partial[key] = partial.get(key, 0) + 1
                    else:
                        reason = str(info.get("reason") or "skipped")[:80]
                        reasons[reason] = reasons.get(reason, 0) + 1
        if blocks:
            out[role] = dict(searched=searched, blocks=blocks, ran=ran, changed=changed,
                             ran_share=round(ran / max(searched, 1), 4),
                             changed_share=round(changed / max(searched, 1), 4),
                             changed_of_ran=round(changed / ran, 4) if ran else None,
                             skipped=dict(sorted(reasons.items(), key=lambda kv: -kv[1])),
                             stopped_early=dict(sorted(partial.items(), key=lambda kv: -kv[1])),
                             layer2_ms=time_stats(ms))
    return out or None


def pair_scores(results: list, capped_as_tie: bool = False) -> np.ndarray:
    """[pairs, 2] scores. Pairs with a capped game are left out, or (``capped_as_tie``) score it 0.5."""
    rows = []
    for r in results:
        scores = [g["score"] for g in r["games"]]
        if all(s is not None for s in scores):
            rows.append(scores)
        elif capped_as_tie:
            rows.append([0.5 if s is None else s for s in scores])
    return np.array(rows, dtype=float).reshape(-1, 2)


def score_line(scores: np.ndarray) -> tuple[float | None, float | None]:
    mean = float(scores.mean()) if len(scores) else None  # None, not NaN: the summary stays valid JSON
    se = float(scores.mean(1).std(ddof=1) / math.sqrt(len(scores))) if len(scores) > 1 else None
    return mean, se


def capped_counts(results: list) -> tuple[int, int]:
    games = sum(1 for r in results for g in r["games"] if g["score"] is None)
    pairs = sum(1 for r in results if any(g["score"] is None for g in r["games"]))
    return games, pairs


def summarize(results: list, seconds: float) -> dict:
    scores = pair_scores(results)
    mean, se = score_line(scores)
    all_scores = pair_scores(results, capped_as_tie=True)
    tie_mean, tie_se = score_line(all_scores)
    capped_games, capped_pairs = capped_counts(results)
    stats = {}
    for r in results:
        for g in r["games"]:
            for k, v in g["stats"].items():
                stats[k] = stats.get(k, 0) + v
    return dict(pairs=len(scores), games=int(scores.size), score=mean, pair_se=se,
                pairs_all=len(all_scores), capped_games=capped_games, capped_pairs=capped_pairs,
                score_capped_as_ties=tie_mean, pair_se_capped_as_ties=tie_se,
                seconds=round(seconds, 1), decisions=stats,
                **worlds_means(results),
                rejected=sum(g["rejected"] for r in results for g in r["games"]),
                unfinished=sum(not g["ended"] for r in results for g in r["games"]),
                timing=timing_summary(results), depth2=depth2_summary(results))


def progress_lines(summary: dict, done: int, total: int) -> list[str]:
    """The log lines for a progress report: score over finished pairs; capped games as ties (when any); times."""
    se = summary["pair_se"]
    means = "  ".join(f"{key} {summary[key]:.2f}" if summary[key] is not None else f"{key} n/a"
                      for key in ("worlds_mean", "opponent_worlds_mean"))
    nan = float("nan")
    score = summary["score"] if summary["score"] is not None else nan
    lines = [f"{done}/{total} pairs  score {score:.3f} ± {se if se is not None else nan:.3f}"
             f"  {means}  ({summary['seconds']:.0f} s)"]
    if summary["capped_games"]:
        tie_se = summary["pair_se_capped_as_ties"]
        lines.append(f"    capped: {summary['capped_games']} games in {summary['capped_pairs']} pairs (left out above); "
                     f"as ties: score {summary['score_capped_as_ties']:.3f} ± "
                     f"{tie_se if tie_se is not None else nan:.3f} over {summary['pairs_all']} pairs")
    for role, t in summary["timing"].items():
        stats = t["search"] or t["all"]
        if stats:
            kind = "searched" if t["search"] else "all"
            lines.append(f"    {role} ms ({kind}, n={stats['count']}): mean {stats['mean']:.0f}  p50 {stats['p50']:.0f}"
                         f"  p90 {stats['p90']:.0f}  p99 {stats['p99']:.0f}  max {stats['max']:.0f}")
    for role, d in (summary["depth2"] or {}).items():
        layer2 = d["layer2_ms"]
        lines.append(f"    {role} depth2: ran {d['ran_share']:.0%}  changed {d['changed_share']:.0%} of searched"
                     f"  skipped {sum(d['skipped'].values())}"
                     + (f"  layer-2 ms mean {layer2['mean']:.0f}" if layer2 else ""))
    return lines


def load_search_arg(text: str):
    """``--search``/``--opponent-search``: "off", a SearchConfig JSON object, or a JSON file. A file or object with a
    ``search`` key is a bot config (tools/scenarios configs): its ``search`` is used, and its ``budget`` (when given)
    is passed to the bot. Returns (search: dict | False, bot keyword arguments)."""
    if text == "off":
        return False, {}
    path = Path(text)
    value = json.loads(path.read_text()) if not text.lstrip().startswith("{") and path.is_file() else json.loads(text)
    if value is False or value == "off":
        return False, {}
    extra = {}
    if isinstance(value, dict) and isinstance(value.get("search"), (dict, bool)):
        if value.get("budget") is not None:
            extra["budget"] = value["budget"]
        value = value["search"]
    return value, extra


def check_depth2(search, bot, label: str) -> None:
    """A ``depth2`` setting must reach the bot: fail when this SearchConfig has no such field (the search2 core
    change is missing), instead of silently evaluating the 1-ply search."""
    if isinstance(search, dict) and "depth2" in search and bot.search is not None \
            and not hasattr(bot.search, "depth2"):
        raise SystemExit(f"{label}: 'depth2' is set but this SearchConfig has no depth2 field "
                         "(training/search.py without the search2 core change)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--checkpoint", required=True)
    parser.add_argument("--value-checkpoint", default=None, help="search bot's leaf values from this checkpoint")
    parser.add_argument("--base-checkpoint", default=None, help="opponent policy (default: the same checkpoint)")
    parser.add_argument("--opponent", choices=("policy", "smart", "search"), default="policy")
    parser.add_argument("--opponent-search", default="{}",
                        help="SearchConfig JSON (or a JSON file) of --opponent search; may include depth2")
    parser.add_argument("--base-temperature", type=float, default=0.5)
    parser.add_argument("--policy-temperature", type=float, default=0.5, help="search bot's fallback policy")
    parser.add_argument("--search", default="{}",
                        help="SearchConfig JSON or a JSON file (may include depth2); 'off' for the policy alone")
    parser.add_argument("--budget-ms", type=float, default=None, help="per-decision search budget in ms (default: off)")
    parser.add_argument("--opponent-budget-ms", type=float, default=None,
                        help="per-decision budget for --opponent search (default: --budget-ms)")
    parser.add_argument("--pool", default=str(REPO / "data/teams/eval-s43-200.txt"))
    parser.add_argument("--set-pool", default=str(REPO / "data/teams/pool-s1-200k.txt.gz"))
    parser.add_argument("--pairs", type=int, default=100)
    parser.add_argument("--first-pair", type=int, default=0)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--concurrency", type=int, default=4)
    parser.add_argument("--max-turns", type=int, default=400, help="turn cap; capped games are reported separately")
    parser.add_argument("--device", default="auto")
    parser.add_argument("--torch-threads", type=int, default=2)
    parser.add_argument("--engine-mask", action="store_true", help="give the search bot the engine's mask (debug)")
    parser.add_argument("--positions-out", default=None,
                        help="JSONL file for a sample of decision points (tools/positions.py), for replays")
    parser.add_argument("--positions-rate", type=float, default=0.05,
                        help="share of decision points (both sides) written to --positions-out")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    opponent_budget_ms = args.opponent_budget_ms if args.opponent_budget_ms is not None else args.budget_ms

    torch.set_num_threads(args.torch_threads)
    search, search_extra = load_search_arg(args.search)
    bot = SearchBot(name="search", checkpoint=args.checkpoint, search=search, pools=[args.set_pool],
                    device=args.device, policy_temperature=args.policy_temperature,
                    value_checkpoint=args.value_checkpoint, **search_extra)
    check_depth2(search, bot, "--search")
    base_net = bot.net
    if args.base_checkpoint and Path(args.base_checkpoint).resolve() != Path(args.checkpoint).resolve():
        from training.search import Net

        base_net = Net(Path(args.base_checkpoint), bot.dex, args.device)
    opponent_bot = None
    if args.opponent == "search":
        opponent_search, opponent_extra = load_search_arg(args.opponent_search)
        opponent_bot = SearchBot(name="opponent", checkpoint=args.base_checkpoint or args.checkpoint,
                                 search=opponent_search, pools=[args.set_pool], device=args.device,
                                 policy_temperature=args.policy_temperature, **opponent_extra)
        check_depth2(opponent_search, opponent_bot, "--opponent-search")
    teams = [line.strip() for line in Path(args.pool).read_text().splitlines() if line.strip()]
    rng = np.random.default_rng(args.seed)
    plan = []
    for p in range(args.first_pair + args.pairs):
        i, j = rng.choice(len(teams), size=2, replace=False)
        seed = tuple(int(x) for x in rng.integers(0, 1 << 16, size=4))
        if p >= args.first_pair:
            plan.append((p, teams[i], teams[j], seed))
    dex_path = str(bot.dex.path)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    positions_path = Path(args.positions_out) if args.positions_out else None
    if positions_path is not None:
        positions_path.parent.mkdir(parents=True, exist_ok=True)
        positions_path.write_text("")
    results: list = []
    lock = threading.Lock()
    started = time.time()

    def run(item):
        p, t1, t2, seed = item
        if RUN_FLAG.exists():
            raise RuntimeError(f"{RUN_FLAG} exists: a training run is active")
        game_rng = np.random.default_rng([args.seed, p])
        if args.opponent == "smart":
            def factory(side, team, seed=None):
                return SmartOpponent(dex_path, side, seed=args.seed * 1000 + p)
        elif args.opponent == "search":
            def factory(side, team, seed=None):
                return SearchOpponent(opponent_bot, side, team, args.engine_mask, opponent_budget_ms, seed=seed)
        else:
            def factory(side, team, seed=None):
                return BasePolicy(base_net, dex_path, side, args.base_temperature, game_rng)
        extra = dict(max_turns=args.max_turns)
        games = []
        for side in (0, 1):  # game g: the search bot on side g; every player gets its own seed
            if positions_path is not None:
                extra.update(positions_rate=args.positions_rate,
                             positions_rng=np.random.default_rng([args.seed, p, side, 99]))
            games.append(play_game(bot, factory, t1, t2, seed, side, args.engine_mask, budget_ms=args.budget_ms,
                                   player_seed=derive_seed(args.seed, p, side, side),
                                   opponent_seed=derive_seed(args.seed, p, side, 1 - side), **extra))
        positions = []
        for g, game in enumerate(games):
            for position in game.pop("positions", []):
                positions.append(dict(position, pair=p, game=g))
        with lock:
            results.append(dict(pair=p, games=games))
            if positions:
                with positions_path.open("a") as f:
                    f.writelines(json.dumps(position) + "\n" for position in positions)
            done = len(results)
            if done % 5 == 0 or done == len(plan):
                summary = save()
                print("\n".join(progress_lines(summary, done, len(plan))), flush=True)

    def save():
        summary = summarize(results, time.time() - started)
        out_path.write_text(json.dumps(dict(args=vars(args), bot=bot.describe(),
                                            opponent_bot=opponent_bot.describe() if opponent_bot else None,
                                            summary=summary, results=sorted(results, key=lambda r: r["pair"])),
                                       indent=1))
        return summary

    with ThreadPoolExecutor(args.concurrency) as pool:
        list(pool.map(run, plan))
    save()
    print(json.dumps(json.loads(out_path.read_text())["summary"], indent=1))


if __name__ == "__main__":
    main()
