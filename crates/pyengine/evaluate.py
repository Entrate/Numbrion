"""Duplicate evaluation of self-play snapshots on held-out teams (docs/training/TIPS.md "Evaluation").

    .venv\\Scripts\\python.exe crates\\pyengine\\evaluate.py --run scratch\\training\\selfplay

Every matchup is played on mirrored battles: the second half of the battles repeats the first half's teams and
battle seeds with the sides swapped, so team luck mostly cancels. Opponents:

* ``random``: a random legal player (switch 10%, Tera 15%, otherwise a uniform legal move),
* ``heuristic``: "always use the strongest attack" from the damage-calc features (no switching or Tera),
* ``smart``: the doubles-aware scripted player of training.smart (Fake Out, focus fire, Protect, Tera, switching),
* earlier snapshots of the same run.

Also reported for the final snapshot: mistake counters from its player-view logs (against the heuristic and against
smart), value calibration of the (oracle) critic, and a few Showdown HTML replays.
"""
from __future__ import annotations

import argparse
import json
import math
import time
from pathlib import Path

import numpy as np
import torch

from training.device import training_device
from training.dex import load_dex, to_id
from training.model import Model
from training.ppo import PASS, sample
from training.smart import PROTECT_MOVES, SmartHeuristicAgent
from training.vecenv import VecEnv

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")
N_ACTIONS = 47


def legal_random(mask: np.ndarray, rng, switch_prob=0.1, tera_prob=0.15) -> np.ndarray:
    """Per row: a switch with ``switch_prob`` if one is legal, else a uniform legal non-Tera move code (Tera with
    ``tera_prob`` when legal), else any legal code."""
    out = np.full(len(mask), PASS, dtype=np.int32)
    codes = np.arange(N_ACTIONS)
    for i, m in enumerate(mask):
        moves = codes[:40][m[:40]]
        switches = codes[40:46][m[40:46]]
        plain = moves[moves % 2 == 0]
        tera = moves[moves % 2 == 1]
        if len(switches) and (rng.random() < switch_prob or not len(moves)):
            out[i] = rng.choice(switches)
        elif len(tera) and rng.random() < tera_prob:
            out[i] = rng.choice(tera)
        elif len(plain):
            out[i] = rng.choice(plain)
        elif m.any():
            out[i] = rng.choice(codes[m])
    return out


class RandomAgent:
    name = "random"

    def __init__(self, rng):
        self.rng = rng

    def first(self, arrays, rows, mask0):
        return legal_random(mask0, self.rng)

    def second(self, rows, mask1):
        return legal_random(mask1, self.rng)


class HeuristicAgent:
    """Always the strongest attack: highest expected damage to foes minus damage to its own side, no Tera."""

    name = "heuristic"

    def __init__(self, rng):
        self.rng = rng

    def _pick(self, mask, features, slot):
        out = legal_random(mask, self.rng, switch_prob=0.0, tera_prob=0.0)
        score = features[:, slot, :, 0] - features[:, slot, :, 1]  # [rows, 40]
        legal = mask[:, :40] & (np.arange(40) % 2 == 0)
        score = np.where(legal, score, -np.inf)
        best = score.argmax(-1)
        good = np.isfinite(score.max(-1)) & (score.max(-1) > 0)
        return np.where(good, best, out).astype(np.int32)

    def first(self, arrays, rows, mask0):
        self.features = arrays["action_features"][rows]
        return self._pick(mask0, self.features, 0)

    def second(self, rows, mask1):
        return self._pick(mask1, self.features, 1)


class PolicyAgent:
    def __init__(self, model, device, rng, name, record_values=False):
        self.model, self.device, self.rng, self.name = model, device, rng, name
        self.record_values = record_values
        self.values: list = []

    def first(self, arrays, rows, mask0):
        up = lambda a: torch.from_numpy(np.ascontiguousarray(a)).to(self.device)
        with torch.no_grad():
            batch = {k: up(arrays[k][rows]) for k in OBS_KEYS}
            self.state = self.model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"],
                                          batch["token_features"])
            lp0, self.vectors0 = self.model.slot0(self.state, up(mask0))
            if self.record_values:
                partner = rows ^ 1
                self.last_values = self.model.value(self.state, up(arrays["ids"][partner]),
                                                    up(arrays["floats"][partner, 0:6])).cpu().numpy()
        self.a0 = sample(lp0.cpu().numpy(), self.rng)
        return self.a0

    def second(self, rows, mask1):
        with torch.no_grad():
            a0 = torch.from_numpy(self.a0.astype(np.int64)).to(self.device)
            lp1 = self.model.slot1(self.state, self.vectors0, a0, torch.from_numpy(mask1).to(self.device))
        return sample(lp1.cpu().numpy(), self.rng)


def load_policy(path: Path, dex, device, rng, name=None, record_values=False) -> PolicyAgent:
    state = torch.load(path, map_location="cpu", weights_only=False)
    model = Model(dex, state.get("width", 128), state.get("layers", 3)).to(device)
    model.load_state_dict(state["model"])
    model.eval()
    return PolicyAgent(model, device, rng, name or path.stem, record_values)


def play(a, b, pool: Path, envs_per_copy: int, rounds: int, seed: int, workers: int, log_games=False):
    """``a`` is side 1 in the first copy and side 2 in the mirrored copy. Returns a's score and details."""
    E = envs_per_copy
    n = 2 * E
    a_side = np.r_[np.zeros(E, dtype=int), np.ones(E, dtype=int)]  # side played by a in each env
    a_rows = 2 * np.arange(n) + a_side
    b_rows = 2 * np.arange(n) + 1 - a_side
    results = [[] for _ in range(n)]  # winner (0/1/-1) of each finished battle, in order
    calibration = []  # (env, battle id, a's value)
    choices = {}  # with log_games: (env, battle id) -> {turn: (a0, a1)} of a's first move decision per turn
    with VecEnv(n, [pool], seed=seed, workers=workers, log_games=log_games, mirror=True) as env:
        obs = env.reset()
        steps = 0
        while min(len(r) for r in results) < rounds and steps < 20000:
            steps += 1
            needs = obs["needs_action"].reshape(-1)
            mask0 = obs["mask0"].reshape(-1, N_ACTIONS).copy()
            mask0[~needs] = False
            mask0[~needs, PASS] = True
            a0 = np.full(2 * n, PASS, dtype=np.int32)
            a0[a_rows] = a.first(obs, a_rows, mask0[a_rows])
            if getattr(a, "record_values", False):
                for env_index, row, value in zip(range(n), a_rows, a.last_values):
                    if needs[row]:
                        calibration.append((env_index, int(obs["battle_id"][env_index]), float(value)))
            a0[b_rows] = b.first(obs, b_rows, mask0[b_rows])
            mask1 = env.mask_slot1(np.where(needs, a0, -1).reshape(n, 2)).reshape(-1, N_ACTIONS).copy()
            mask1[~needs] = False
            mask1[~needs, PASS] = True
            a1 = np.full(2 * n, PASS, dtype=np.int32)
            a1[a_rows] = a.second(a_rows, mask1[a_rows])
            a1[b_rows] = b.second(b_rows, mask1[b_rows])
            actions = np.stack((a0, a1), -1).reshape(n, 2, 2)
            actions[~obs["needs_action"]] = -1
            if log_games:
                for env_index, row in enumerate(a_rows):
                    if needs[row] and min(a0[row], a1[row]) < 40:
                        turns = choices.setdefault((env_index, int(obs["battle_id"][env_index])), {})
                        turns.setdefault(int(obs["turn"][env_index]), (int(a0[row]), int(a1[row])))
            obs = env.step(actions)
            for e in np.flatnonzero(obs["done"]):
                results[e].append(int(obs["winner"][e]))
        logs = env.finished_logs() if log_games else []
    for item in logs:
        if item["side"] == a_side[item["env"]]:
            item["choices"] = choices.get((item["env"], item["battle"]), {})
    games = wins = ties = 0
    paired = []
    outcome = {}
    for e in range(n):
        for k, winner in enumerate(results[e][:rounds]):
            games += 1
            score = 0.5 if winner < 0 else float(winner == a_side[e])
            wins += score == 1.0
            ties += score == 0.5
            outcome[(e, k + 1)] = score
    for e in range(E):  # duplicate pairs: same teams and seed, sides swapped
        for k in range(min(rounds, len(results[e]), len(results[e + E]))):
            paired.append(outcome[(e, k + 1)] + outcome[(e + E, k + 1)])
    score = (wins + 0.5 * ties) / max(games, 1)
    values = [(v, outcome[(e, battle)]) for e, battle, v in calibration if (e, battle) in outcome]
    return dict(a=a.name, b=b.name, games=games, score=score, wins=wins, ties=ties,
                pair_sweeps=sum(p == 2 for p in paired), pair_splits=sum(p == 1 for p in paired),
                pair_losses=sum(p == 0 for p in paired), steps=steps), values, logs, a_side


def elo(score: float, games: int) -> tuple[float, float]:
    p = min(max(score, 0.5 / games), 1 - 0.5 / games)
    error = math.sqrt(p * (1 - p) / games)
    to_elo = lambda x: 400 * math.log10(x / (1 - x))
    return to_elo(p), (to_elo(min(p + 2 * error, 0.999)) - to_elo(max(p - 2 * error, 0.001))) / 2


def mistakes(logs: list, dex) -> dict:
    """Counters over the evaluated player's own decisions in its player-view logs.

    Besides the basic counters: attacks at a foe that could Protect (it used a Protect move in an earlier turn and
    not in the turn before) and how many of them were blocked; own Protect uses, blocks and consecutive-turn
    repeats; switches split into switch actions at the start of a turn, pivots (after a move in the same turn:
    U-turn, Eject Button) and replacements of fainted Pokemon. Logs with the player's action codes per turn
    (``choices``, recorded by ``play``) also give focus fire (both slots chose single-target attacks at the same
    foe) and overkill (an attack chosen at a foe that had already fainted that turn, which then retargets or fails
    with ``[notarget]``).
    """
    counts = dict(moves=0, attacks=0, into_protect=0, fake_out=0, fake_out_failed=0, no_target=0, ally_hits=0,
                  tera=0, switches=0, battles=len(logs), turns=0, attacks_vs_protector=0,
                  into_protect_vs_protector=0, protects=0, protect_blocks=0, protect_repeats=0, switch_actions=0,
                  pivots=0, replacements=0, chosen_attacks=0, focus_fire=0, overkill=0, overkill_retarget=0,
                  overkill_notarget=0)
    damaging = {m["id"] for m in dex.moves if m["category"] != "Status"}
    protecting = set(PROTECT_MOVES)
    for item in logs:
        own = f"p{item['side'] + 1}"
        foe = "p2" if own == "p1" else "p1"
        lines = item["log"]
        choices = item.get("choices")
        turn, moved = 0, False
        protect_seen: dict = {}  # foe name -> first turn it used a Protect move
        protect_turn: dict = {}  # (side, name) -> last turn it used a Protect move
        fainted = set()  # own positions whose Pokemon fainted and was not replaced yet
        foe_down = set()  # foe positions that fainted this turn
        acted: dict = {}  # own slot -> target code of its single-target attack this turn
        for i, line in enumerate(lines):
            parts = line.split("|")
            if len(parts) < 3:
                continue
            event = parts[1]
            if event == "turn":
                turn, moved = int(parts[2]), False
                foe_down, acted = set(), {}
                counts["turns"] += 1
                continue
            if event == "-terastallize" and parts[2].startswith(own):
                counts["tera"] += 1
            if event == "faint":
                (fainted if parts[2].startswith(own) else foe_down).add(parts[2][:3])
            if event == "-activate" and parts[2].startswith(own) and line.endswith("|move: Protect"):
                counts["protect_blocks"] += 1
            if event in ("switch", "drag") and parts[2].startswith(own):
                counts["switches"] += event == "switch" and i > 30
                position = parts[2][:3]
                if turn == 0:
                    pass  # leads
                elif position in fainted:
                    counts["replacements"] += 1
                elif event == "switch":
                    counts["pivots" if moved else "switch_actions"] += 1
                fainted.discard(position)
            if event != "move":
                continue
            moved = True
            move = to_id(parts[3])
            name = parts[2].split(": ", 1)[-1]
            if move in protecting:
                key = (parts[2][:2], name)
                if parts[2].startswith(own):
                    counts["protects"] += 1
                    counts["protect_repeats"] += protect_turn.get(key) == turn - 1
                else:
                    protect_seen.setdefault(name, turn)
                protect_turn[key] = turn
            if not parts[2].startswith(own):
                continue
            counts["moves"] += 1
            follow = []
            for later in lines[i + 1:i + 12]:
                if later.startswith("|move|") or later == "|":
                    break
                follow.append(later)
            if "[notarget]" in line:
                counts["no_target"] += 1
            if move == "fakeout":
                counts["fake_out"] += 1
                counts["fake_out_failed"] += any(f.startswith("|-fail|") for f in follow)
            if move not in damaging:
                continue
            foe_target = len(parts) > 4 and parts[4][:2] not in ("", own)
            blocked = foe_target and any(
                f.startswith("|-activate|") and "move: Protect" in f and not f.split("|")[2].startswith(own)
                for f in follow)
            counts["attacks"] += foe_target
            counts["into_protect"] += blocked
            if foe_target:
                target = parts[4].split(": ", 1)[-1]
                could = protect_seen.get(target, turn) < turn and protect_turn.get((foe, target)) != turn - 1
                counts["attacks_vs_protector"] += could
                counts["into_protect_vs_protector"] += could and blocked
            counts["ally_hits"] += any(f.startswith(f"|-damage|{own}") and "[from]" not in f and
                                       f.split("|")[2] != parts[2] for f in follow)
            choice = choices.get(turn) if choices else None
            slot = "ab".find(parts[2][2:3])
            if choice is None or slot < 0 or slot in acted or "[from]" in line or choice[slot] >= 40:
                continue
            target_code = (choice[slot] % 10) // 2
            if target_code not in (3, 4):
                continue
            acted[slot] = target_code
            counts["chosen_attacks"] += 1
            counts["focus_fire"] += acted.get(1 - slot) == target_code
            if foe + "ab"[target_code - 3] in foe_down:
                counts["overkill"] += 1
                counts["overkill_notarget" if "[notarget]" in line else "overkill_retarget"] += 1
    battles = max(counts["battles"], 1)
    rates = dict(
        into_protect_per_attack=counts["into_protect"] / max(counts["attacks"], 1),
        into_protect_per_attack_vs_protector=(counts["into_protect_vs_protector"]
                                              / max(counts["attacks_vs_protector"], 1)),
        fake_out_fail_rate=counts["fake_out_failed"] / max(counts["fake_out"], 1),
        no_target_per_move=counts["no_target"] / max(counts["moves"], 1),
        ally_hits_per_move=counts["ally_hits"] / max(counts["moves"], 1),
        tera_per_battle=counts["tera"] / battles,
        switches_per_battle=counts["switches"] / battles,
        protects_per_battle=counts["protects"] / battles,
        fake_outs_per_battle=counts["fake_out"] / battles,
        switch_actions_per_battle=counts["switch_actions"] / battles,
        pivots_per_battle=counts["pivots"] / battles,
        replacements_per_battle=counts["replacements"] / battles,
        overkill_per_chosen_attack=counts["overkill"] / max(counts["chosen_attacks"], 1),
        focus_fire_per_chosen_attack=counts["focus_fire"] / max(counts["chosen_attacks"], 1),
    )
    return {**counts, **{k: round(v, 4) for k, v in rates.items()}}


def calibration_table(values: list) -> list:
    if not values:
        return []
    v = np.array([x for x, _ in values])
    outcome = np.array([y for _, y in values])
    table = []
    edges = np.linspace(-1, 1, 11)
    for lo, hi in zip(edges[:-1], edges[1:]):
        inside = (v >= lo) & (v < hi) if hi < 1 else (v >= lo)
        if inside.sum() >= 20:
            predicted = (v[inside].mean() + 1) / 2
            table.append(dict(value_bin=f"[{lo:+.1f},{hi:+.1f})", decisions=int(inside.sum()),
                              predicted_win=round(float(predicted), 3), actual_win=round(float(outcome[inside].mean()), 3)))
    return table


REPLAY = """<!DOCTYPE html>
<meta charset="utf-8" />
<title>{title}</title>
<div class="wrapper replay-wrapper" style="max-width:1180px;margin:0 auto">
<div class="battle"></div><div class="battle-log"></div><div class="replay-controls"></div><div class="replay-controls-2"></div>
<h1 style="font-weight:normal;text-align:center"><strong>{title}</strong></h1>
<script type="text/plain" class="battle-log-data">{log}</script>
</div>
<script>
let daily = Math.floor(Date.now()/1000/60/60/24);document.write('<script src="https://play.pokemonshowdown.com/js/replay-embed.js?version'+daily+'"></'+'script>');
</script>
"""


def write_replays(logs: list, a_side, directory: Path, prefix: str, title: str, count: int) -> list:
    directory.mkdir(parents=True, exist_ok=True)
    written = []
    for item in logs:
        if len(written) >= count:
            break
        if item["side"] != a_side[item["env"]] or item["side"] != 0:
            continue  # the evaluated player's own view, as player 1
        log = "\n".join(line for line in item["log"] if not line.startswith("|t:|")).replace("</", "<\\/")
        outcome = "won" if item["winner"] == item["side"] else ("tie" if item["winner"] < 0 else "lost")
        path = directory / f"{prefix}-{len(written) + 1}-{outcome}.html"
        path.write_text(REPLAY.format(title=f"{title} ({outcome})", log=log), encoding="utf-8")
        written.append(str(path))
    return written


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--run", type=Path, default=REPO / "scratch/training/selfplay")
    parser.add_argument("--pool", type=Path, default=REPO / "data/teams/eval-s43-200.txt")
    parser.add_argument("--device", choices=["auto", "directml", "cuda", "cpu"], default="auto")
    parser.add_argument("--envs", type=int, default=50, help="battles per mirrored copy at a time")
    parser.add_argument("--rounds", type=int, default=4, help="battles per env: games = 2 * envs * rounds")
    parser.add_argument("--final-rounds", type=int, default=10, help="rounds for the final snapshot's baselines")
    parser.add_argument("--every", type=int, default=10, help="evaluate snapshots every N minutes")
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--replays", type=int, default=4)
    parser.add_argument("--seed", type=int, default=7)
    args = parser.parse_args()
    torch.set_num_threads(2)
    device, _ = training_device(args.device)
    dex = load_dex()
    rng = np.random.default_rng(args.seed)
    snapshots = sorted((args.run / "snapshots").glob("m*.pt"))
    final = snapshots[-1]
    chosen = [s for s in snapshots if int(s.stem[1:]) % args.every == 0 and s != final] + [final]
    out = dict(run=str(args.run), pool=str(args.pool), matchups=[], started=time.strftime("%Y-%m-%d %H:%M:%S"))
    output = args.run / "evaluation.json"

    def record(result):
        result["elo"], result["elo_error"] = (round(x) for x in elo(result["score"], result["games"]))
        out["matchups"].append(result)
        print(json.dumps(result), flush=True)
        output.write_text(json.dumps(out, indent=1))

    seed = args.seed
    smart_seed = args.seed + 500  # the smart matchups have their own seeds, so the older ones keep theirs
    random_agent, heuristic = RandomAgent(rng), HeuristicAgent(rng)
    smart = SmartHeuristicAgent(rng, dex)
    result, *_ = play(heuristic, random_agent, args.pool, args.envs, args.rounds, seed, args.workers)
    record(result)
    for opponent in (heuristic, random_agent):  # sanity rows for the smart baseline
        smart_seed += 1
        result, _, logs, a_side = play(smart, opponent, args.pool, args.envs, args.rounds, smart_seed, args.workers,
                                       log_games=opponent is heuristic)
        record(result)
        if logs:
            out["smart_mistakes"] = mistakes([x for x in logs if x["side"] == a_side[x["env"]]], dex)
    for snapshot in chosen:
        policy = load_policy(snapshot, dex, device, rng)
        rounds = args.final_rounds if snapshot == final else args.rounds
        for opponent in (random_agent, heuristic, smart):
            if opponent is smart:
                smart_seed += 1
            else:
                seed += 1
            is_final = snapshot == final and opponent is not random_agent
            policy.record_values = snapshot == final and opponent is heuristic
            result, values, logs, a_side = play(policy, opponent, args.pool, args.envs, rounds,
                                                smart_seed if opponent is smart else seed, args.workers,
                                                log_games=is_final)
            result["minutes"] = int(snapshot.stem[1:])
            record(result)
            if is_final:
                own_logs = [x for x in logs if x["side"] == a_side[x["env"]]]
                foe_logs = [x for x in logs if x["side"] != a_side[x["env"]]]
                if opponent is smart:
                    out["final_mistakes_vs_smart"] = mistakes(own_logs, dex)
                    out["smart_replays"] = write_replays(logs, a_side, args.run / "replays", "final-vs-smart",
                                                         f"{final.stem} vs smart heuristic", args.replays)
                    continue
                out["final_mistakes"] = mistakes(own_logs, dex)
                out["heuristic_mistakes"] = mistakes(foe_logs, dex)
                out["calibration"] = calibration_table(values)
                out["replays"] = write_replays(logs, a_side, args.run / "replays", "final-vs-heuristic",
                                               f"{final.stem} vs strongest-attack heuristic", args.replays)
    final_policy = load_policy(final, dex, device, rng, name=final.stem)
    for snapshot in chosen[:-1]:
        seed += 1
        older = load_policy(snapshot, dex, device, rng)
        result, *_ = play(final_policy, older, args.pool, args.envs, args.rounds, seed, args.workers)
        result["minutes"] = int(snapshot.stem[1:])
        record(result)
    out["finished"] = time.strftime("%Y-%m-%d %H:%M:%S")
    output.write_text(json.dumps(out, indent=1))
    print(f"Wrote {output}", flush=True)


if __name__ == "__main__":
    main()
