"""Does search beat the plain policy? Mirrored games between a SearchBot and a policy (or the smart player).

    .venv/bin/python crates/pyengine/search_eval.py --checkpoint CKPT --pairs 200 --out scratch/search-eval/run.json

Every pair is two battles with the same teams and battle seed, the search bot on side 1 in the first and side 2 in
the second, so team and seed luck cancel (as evaluate.py's duplicate games). The search bot is protocol-driven
(training.player): it sees only its player-view lines and requests and builds its own worlds. The opponent is the
base policy as evaluate.py/compare.py play it: temperature ``--base-temperature`` (0.5), MoveGuards, the engine's
legal masks. ``--opponent smart`` plays Rust's smart scripted player instead.

Score = the search bot's mean result (win 1, tie 0.5); the error is the standard error of the pair means. Games run
``--concurrency`` at a time in threads (the engine, world builder and encoder release the GIL).
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
    """Another SearchBot (e.g. a different budget) as the opponent, driven like the evaluated bot."""

    def __init__(self, bot: SearchBot, side: int, team: str, engine_mask: bool, budget_ms: float | None = None):
        self.side, self.engine_mask = side, engine_mask
        self.budget_ms = budget_ms
        self.worlds: list[int] = []
        self.player = bot.new_player("opponent")
        self.player.team = team

    def feed(self, lines):
        self.player.feed(lines)

    def act(self, battle) -> tuple[int, int]:
        self.player.set_request(battle.request_json(self.side))
        if self.engine_mask:
            self.player.mask_hint = battle.legal_mask(self.side)
        self.player.budget_ms = self.budget_ms
        decision = self.player.decide()
        if decision.analysis.get("source") == "search":
            self.worlds.append(decision.analysis["worlds"])
        return decision.a0, decision.a1


def play_game(bot: SearchBot, opponent_factory, team1: str, team2: str, seed: tuple, search_side: int,
              engine_mask: bool, max_turns: int = 400, budget_ms: float | None = None) -> dict:
    battle = numbrion.Battle(seed, p1=team1, p2=team2, log=True)
    battle.start()
    player = bot.new_player(f"eval-{seed}")
    player.team = team1 if search_side == 0 else team2
    opponent = opponent_factory(1 - search_side, team2 if search_side == 0 else team1)
    rejected = 0
    worlds: list[int] = []
    while not battle.ended and battle.turn <= max_turns:
        lines = [battle.drain_player_log(s) for s in (0, 1)]
        player.feed(lines[search_side])
        opponent.feed(lines[1 - search_side])
        order = [s for s in (0, 1) if battle.needs_action(s)]
        for side in order:
            if side == search_side:
                player.set_request(battle.request_json(side))
                for attempt in range(3):
                    if engine_mask:
                        player.mask_hint = battle.legal_mask(side)
                    player.budget_ms = budget_ms
                    decision = player.decide()
                    if decision is None:
                        break
                    if decision.analysis.get("source") == "search":
                        worlds.append(decision.analysis["worlds"])
                    try:
                        battle.choose_action(side, decision.a0, decision.a1)
                        break
                    except numbrion.ChoiceError as error:
                        rejected += 1
                        player.rejected(str(error.args[0]))
                        if len(error.args) > 1 and error.args[1]:
                            player.set_request(error.args[1])
                else:
                    battle.choose(side, "default")
            else:
                a0, a1 = opponent.act(battle)
                battle.choose_action(side, a0, a1)
    # A battle stopped at the turn cap has no result (None): its pair is left out of the score, not counted as a tie.
    score = None if not battle.ended else 0.5 if battle.tie or battle.winner is None else float(battle.winner == search_side)
    opponent_worlds = opponent.worlds if isinstance(opponent, SearchOpponent) else []
    return dict(score=score, turns=battle.turn, ended=battle.ended, rejected=rejected, stats=dict(player.stats),
                search_side=search_side, worlds_count=len(worlds), worlds_mean=float(np.mean(worlds)) if worlds else None,
                opponent_worlds_count=len(opponent_worlds),
                opponent_worlds_mean=float(np.mean(opponent_worlds)) if opponent_worlds else None)


def worlds_means(results: list) -> dict:
    """Decision-weighted means across games; policy/forced decisions do not contribute."""
    means = {}
    for prefix in ("", "opponent_"):
        count = sum(g[f"{prefix}worlds_count"] for r in results for g in r["games"])
        total = sum((g[f"{prefix}worlds_mean"] or 0) * g[f"{prefix}worlds_count"]
                    for r in results for g in r["games"])
        means[f"{prefix}worlds_mean"] = total / count if count else None
    return means


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--checkpoint", required=True)
    parser.add_argument("--value-checkpoint", default=None, help="search bot's leaf values from this checkpoint")
    parser.add_argument("--base-checkpoint", default=None, help="opponent policy (default: the same checkpoint)")
    parser.add_argument("--opponent", choices=("policy", "smart", "search"), default="policy")
    parser.add_argument("--opponent-search", default="{}", help="SearchConfig JSON of --opponent search")
    parser.add_argument("--base-temperature", type=float, default=0.5)
    parser.add_argument("--policy-temperature", type=float, default=0.5, help="search bot's fallback policy")
    parser.add_argument("--search", default="{}", help="SearchConfig JSON; 'off' for the policy alone")
    parser.add_argument("--budget-ms", type=float, default=None, help="per-decision search budget in ms (default: off)")
    parser.add_argument("--opponent-budget-ms", type=float, default=None,
                        help="per-decision budget for --opponent search (default: --budget-ms)")
    parser.add_argument("--pool", default=str(REPO / "data/teams/eval-s43-200.txt"))
    parser.add_argument("--set-pool", default=str(REPO / "data/teams/pool-s1-200k.txt.gz"))
    parser.add_argument("--pairs", type=int, default=100)
    parser.add_argument("--first-pair", type=int, default=0)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--concurrency", type=int, default=4)
    parser.add_argument("--device", default="auto")
    parser.add_argument("--torch-threads", type=int, default=2)
    parser.add_argument("--engine-mask", action="store_true", help="give the search bot the engine's mask (debug)")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    opponent_budget_ms = args.opponent_budget_ms if args.opponent_budget_ms is not None else args.budget_ms

    torch.set_num_threads(args.torch_threads)
    search = False if args.search == "off" else json.loads(args.search)
    bot = SearchBot(name="search", checkpoint=args.checkpoint, search=search, pools=[args.set_pool],
                    device=args.device, policy_temperature=args.policy_temperature,
                    value_checkpoint=args.value_checkpoint)
    base_net = bot.net
    if args.base_checkpoint and Path(args.base_checkpoint).resolve() != Path(args.checkpoint).resolve():
        from training.search import Net

        base_net = Net(Path(args.base_checkpoint), bot.dex, args.device)
    opponent_bot = None
    if args.opponent == "search":
        opponent_bot = SearchBot(name="opponent", checkpoint=args.base_checkpoint or args.checkpoint,
                                 search=json.loads(args.opponent_search), pools=[args.set_pool], device=args.device,
                                 policy_temperature=args.policy_temperature)
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
    results: list = []
    lock = threading.Lock()
    started = time.time()

    def run(item):
        p, t1, t2, seed = item
        if RUN_FLAG.exists():
            raise RuntimeError(f"{RUN_FLAG} exists: a training run is active")
        game_rng = np.random.default_rng([args.seed, p])
        if args.opponent == "smart":
            factory = lambda side, team: SmartOpponent(dex_path, side, seed=args.seed * 1000 + p)  # noqa: E731
        elif args.opponent == "search":
            factory = lambda side, team: SearchOpponent(opponent_bot, side, team, args.engine_mask,  # noqa: E731
                                                        opponent_budget_ms)
        else:
            factory = lambda side, team: BasePolicy(base_net, dex_path, side, args.base_temperature,  # noqa: E731
                                                    game_rng)
        games = [play_game(bot, factory, t1, t2, seed, side, args.engine_mask, budget_ms=args.budget_ms)
                 for side in (0, 1)]
        with lock:
            results.append(dict(pair=p, games=games))
            done = len(results)
            scores = finished(results)
            mean = scores.mean() if len(scores) else float("nan")
            se = scores.mean(1).std(ddof=1) / math.sqrt(len(scores)) if len(scores) > 1 else float("nan")
            if done % 5 == 0 or done == len(plan):
                means = worlds_means(results)
                worlds_text = "  ".join(f"{key} {value:.2f}" if value is not None else f"{key} n/a"
                                         for key, value in means.items())
                print(f"{done}/{len(plan)} pairs  score {mean:.3f} ± {se:.3f}  {worlds_text}"
                      f"  ({time.time() - started:.0f} s)",
                      flush=True)
                save()

    def finished(results):
        return np.array([[g["score"] for g in r["games"]] for r in results
                         if all(g["score"] is not None for g in r["games"])]).reshape(-1, 2)

    def save():
        scores = finished(results)
        stats = {}
        for r in results:
            for g in r["games"]:
                for k, v in g["stats"].items():
                    stats[k] = stats.get(k, 0) + v
        summary = dict(pairs=len(scores), games=int(scores.size), score=float(scores.mean()),
                       pair_se=float(scores.mean(1).std(ddof=1) / math.sqrt(len(scores))) if len(scores) > 1 else None,
                       seconds=round(time.time() - started, 1), decisions=stats,
                       **worlds_means(results),
                       rejected=sum(g["rejected"] for r in results for g in r["games"]),
                       unfinished=sum(not g["ended"] for r in results for g in r["games"]))
        out_path.write_text(json.dumps(dict(args=vars(args), bot=bot.describe(), summary=summary,
                                            results=sorted(results, key=lambda r: r["pair"])), indent=1))

    with ThreadPoolExecutor(args.concurrency) as pool:
        list(pool.map(run, plan))
    save()
    print(json.dumps(json.loads(out_path.read_text())["summary"], indent=1))


if __name__ == "__main__":
    main()
