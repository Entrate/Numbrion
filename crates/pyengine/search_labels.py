"""Search labels for policy distillation (train_selfplay.py ``--distill``): search-vs-search games where every searched
decision is stored with the search's mixed strategy over its candidate pairs.

    .venv/bin/python crates/pyengine/search_labels.py --checkpoint BEST.pt --minutes 60 --out scratch/distill/NAME

Both sides of every game are SearchPlayers of ``--checkpoint`` (the default SearchConfig unless ``--search``), driven
in-process like search_eval.py: each sees only its own player-view lines and requests. A decision that searched
(not forced, not a policy fallback) becomes one label: the player's encoded observation row (training.encoder, the
same rows the policy reads), the guarded slot-a mask, the candidate pairs (slot a, slot b) with the guarded slot-b mask
of each pair's slot a, the search's strategy ``x`` and pair values ``q``, and the policy's log-prior of each pair.

``--explore``: the share of decisions where the side plays a pair sampled from the policy at temperature 1 instead of
the search's choice (the label is still the search's), so the labelled positions also cover worse play than the
search's own. Labels are written as ``labels-NNNN.npz`` shards of ``--shard`` rows plus ``games.jsonl``.
"""
from __future__ import annotations

import argparse
import gzip
import json
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

from training.guards import MoveGuards  # noqa: E402
from training.protocol import choice_text  # noqa: E402
from training.search import N_ACTIONS, OBS_KEYS, SearchBot  # noqa: E402

GUARDS = MoveGuards()


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
    labels, explored, rejected = [], 0, 0
    while not battle.ended and battle.turn <= max_turns:
        lines = [battle.drain_player_log(s) for s in (0, 1)]
        for side in (0, 1):
            players[side].feed(lines[side])
        for side in [s for s in (0, 1) if battle.needs_action(s)]:
            player = players[side]
            player.set_request(battle.request_json(side))
            for _ in range(3):
                decision = player.decide()
                if decision is None:
                    battle.choose(side, "default")
                    break
                a0, a1 = decision.a0, decision.a1
                if player.last_search is not None:
                    labels.append(make_label(player.last_search, k))
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
    winner = None if not battle.ended else (-1 if battle.tie or battle.winner is None else int(battle.winner))
    stats = {key: sum(p.stats.get(key, 0) for p in players) for key in ("search", "policy", "forced", "no_worlds")}
    return labels, dict(turns=battle.turn, ended=battle.ended, winner=winner, labels=len(labels), explored=explored,
                        rejected=rejected, **stats)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--checkpoint", required=True)
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

    torch.set_num_threads(args.torch_threads)
    search = json.loads(args.search)
    teams = read_teams(Path(args.pool))
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "config.json").write_text(json.dumps(vars(args), default=str, indent=1))
    local = threading.local()
    lock = threading.Lock()
    pending: list[dict] = []
    totals = dict(games=0, labels=0, shards=0)
    started = time.time()
    deadline = started + args.minutes * 60

    def bots():
        if not hasattr(local, "bots"):  # per thread: no policy-lock contention between games
            local.bots = [SearchBot(name=f"label{s}", checkpoint=args.checkpoint, search=search, pools=[args.set_pool],
                                    device=args.device, policy_temperature=args.policy_temperature) for s in (0, 1)]
            for bot in local.bots:
                bot.rng = np.random.default_rng([args.seed, threading.get_ident() % (1 << 31), len(local.bots)])
        return local.bots

    def flush(final=False):
        while len(pending) >= args.shard or (final and pending):
            rows, pending[:] = pending[:args.shard], pending[args.shard:]
            arrays = {key: np.stack([row[key] for row in rows]) for key in rows[0]}
            np.savez_compressed(args.out / f"labels-{totals['shards']:04d}.npz", **arrays)
            totals["shards"] += 1

    def run(index: int):
        if time.time() > deadline or (args.games and index >= args.games):
            return
        game_rng = np.random.default_rng([args.seed, index])
        i, j = game_rng.choice(len(teams), size=2, replace=False)
        seed = tuple(int(x) for x in game_rng.integers(0, 1 << 16, size=4))
        k = (search.get("ours") or 8)
        labels, info = play_game(bots(), teams[i], teams[j], seed, args.explore, game_rng, k)
        with lock:
            pending.extend(labels)
            totals["games"] += 1
            totals["labels"] += len(labels)
            with (args.out / "games.jsonl").open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(dict(game=index, **info)) + "\n")
            flush()
            if totals["games"] % 20 == 0:
                elapsed = time.time() - started
                print(f"{totals['games']} games, {totals['labels']} labels ({totals['labels'] / elapsed:.1f}/s, "
                      f"{elapsed / 60:.1f} min)", flush=True)

    limit = args.games or 10 ** 9
    with ThreadPoolExecutor(args.concurrency) as pool:
        futures = []
        index = 0
        while index < limit and time.time() <= deadline:
            futures = [f for f in futures if not f.done()]
            while len(futures) < 2 * args.concurrency and index < limit:
                futures.append(pool.submit(run, index))
                index += 1
            for future in [f for f in futures if f.done()]:
                future.result()
            time.sleep(0.2)
        for future in futures:
            future.result()
    with lock:
        flush(final=True)
    print(f"Done: {totals['games']} games, {totals['labels']} labels in {totals['shards']} shards, "
          f"{(time.time() - started) / 60:.1f} min", flush=True)


if __name__ == "__main__":
    main()
