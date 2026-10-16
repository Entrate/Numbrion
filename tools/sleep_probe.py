"""Sleep-cost probe: how much the value head thinks sleep costs, against what it costs in playouts.

    .venv/bin/python tools/sleep_probe.py --checkpoint SPARK.pt --positions 200 --playouts 16 --out scratch/sleep-probe

Positions: self-play games of ``--checkpoint``'s policy (both sides, ``--temperature``) on ``--pool`` teams. In each
game one boundary is drawn among those where a side S has an active Pokemon X that Spore and Hypnosis could put to
sleep (training.sleep's effective-target rules, with the opposing user's ability) and the opposing side has an active
Pokemon Y with a move slot it has not used so far.

The engine cannot set a status directly, so the pair of positions is made by playing a sleep move against X with a
fixed seed. The game is replayed from its start twice with the same seed and choices, Y's unused move slot replaced
by Spore (copy A) or by Hypnosis (copy B); both replays must reproduce the original players' logs exactly (else the
position is skipped). At the chosen boundary both copies are reseeded with the same seed and play the same turn: S
plays one policy sample, Y uses its new move on X, Y's partner plays one policy sample. Spore (100% accuracy) and
Hypnosis (60%) consume the same accuracy draw, so seeds are tried until Hypnosis misses while Spore puts X to sleep:
up to that draw the two copies are identical, and at the next boundary X is asleep in A and awake in B. What remains
different besides the sleep: the PRNG stream after the accuracy draw (Spore's sleep-turn draw) and which move Y
revealed. S's view of the position is what the head scores; the opponent's own row is its partner input.

Reported per position (JSONL) and on average with standard errors over positions, all from S's perspective:
(a) head cost = V(awake, B) - V(asleep, A), the value head's estimate at the next boundary (``--checkpoint``'s head,
plus every ``--heads`` checkpoint); (b) empirical cost = mean playout result from B - from A, over ``--playouts``
playouts each with the policy on both sides; playout k of A and of B share the battle seed and the sampling seed.
Results are +1 / 0 / -1 (``empirical_cost``) and also discounted by ``--gamma`` per boundary
(``empirical_cost_discounted``, the value head's target scale). Playouts that pass ``--max-turns`` count as ties.
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import numpy as np
import torch

REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / "crates/pyengine"))

import numbrion  # noqa: E402

from training.dex import load_dex, to_id  # noqa: E402
from training.encoder import Buffers  # noqa: E402
from training.guards import MoveGuards  # noqa: E402
from training.model import Model  # noqa: E402
from training.sleep import SleepMoves  # noqa: E402

OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")


def load_model(path: Path, dex) -> Model:
    state = torch.load(path, map_location="cpu", weights_only=False)
    return Model.from_state(dex, state["model"], state.get("width", 128), state.get("layers", 3)).eval()


class Players:
    """Policy play for many battles at once: an ObsEncoder with rows 2b + side, guarded masks, tempered sampling."""

    def __init__(self, model: Model, dex, battles: int, temperature: float):
        self.model, self.temperature = model, temperature
        self.encoder = numbrion.ObsEncoder(str(dex.path), 2 * battles)
        self.buffers = Buffers(2 * battles)
        self.guards = MoveGuards()

    def outputs(self):
        b = self.buffers
        return b.ids, b.floats, b.field, b.foe_match, b.fainted, b.guard, b.action_features, b.token_features

    def encode(self, rows, battles):
        """Encode rows 2b + side from ``battles[b]``'s request and new player-view lines."""
        # Every tracker here (restored or not) belongs to battle id 1: a new id would reset it.
        observations = [dict(battle_id=1, request=battles[r // 2].request_json(r % 2),
                             log=battles[r // 2].drain_player_log(r % 2)) for r in rows]
        self.encoder.encode_rows(list(map(int, rows)), observations, *self.outputs())

    @torch.no_grad()
    def act(self, rows, battles, rng, fixed: dict | None = None) -> dict:
        """{row: (a0, a1)} for encoded rows 2b + side of ``battles[b]``. ``rng``: one Generator, or one per battle
        (each row's sampling noise then comes from its battle's own stream). ``fixed`` {row: (slot, code)} pins one
        slot (the other is sampled given it: slot b conditioned on the pinned slot-a code, or slot a among codes
        jointly legal with the pinned b)."""
        fixed = fixed or {}
        rows = np.asarray(rows, dtype=np.int64)
        b = self.buffers
        x = {k: torch.from_numpy(np.ascontiguousarray(getattr(b, k)[rows])) for k in OBS_KEYS}
        state = self.model.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
        legal = [battles[r // 2].legal_mask(r % 2) for r in rows]
        mask0 = np.stack([m[0] for m in legal])
        for i, r in enumerate(rows):
            if int(r) in fixed:
                slot, code = fixed[int(r)]
                mask0[i] = False
                if slot == 0:
                    mask0[i, code] = True
                else:
                    mask0[i] = legal[i][1][:, code]
        mask0, _ = self.guards.apply(mask0, b.guard[rows], 0)
        lp0, vectors0 = self.model.slot0(state, torch.from_numpy(mask0))
        a0 = self._sample(lp0.numpy(), mask0, rows, rng)
        mask1 = np.stack([battles[r // 2].legal_mask_slot1(r % 2, int(a)) for r, a in zip(rows, a0)])
        for i, r in enumerate(rows):
            if int(r) in fixed and fixed[int(r)][0] == 1:
                code = fixed[int(r)][1]
                mask1[i] = False
                mask1[i, code] = True
        mask1, _ = self.guards.apply(mask1, b.guard[rows], 1)
        lp1 = self.model.slot1(state, vectors0, torch.from_numpy(a0), torch.from_numpy(mask1))
        a1 = self._sample(lp1.numpy(), mask1, rows, rng)
        return {int(r): (int(p), int(q)) for r, p, q in zip(rows, a0, a1)}

    def _sample(self, logp, mask, rows, rng):
        if self.temperature <= 0:
            return np.where(mask, logp, -np.inf).argmax(1)
        if isinstance(rng, (list, tuple)):
            noise = np.stack([rng[r // 2].gumbel(size=logp.shape[1]) for r in rows])
        else:
            noise = rng.gumbel(size=logp.shape)
        return (np.where(mask, logp / self.temperature, -np.inf) + noise).argmax(1)


@torch.no_grad()
def values(model: Model, b: Buffers, rows) -> list[float]:
    rows = np.asarray(rows, dtype=np.int64)
    x = {k: torch.from_numpy(np.ascontiguousarray(getattr(b, k)[rows])) for k in OBS_KEYS}
    state = model.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
    partner = rows ^ 1
    v = model.value(state, torch.from_numpy(b.ids[partner]), torch.from_numpy(b.floats[partner, 0:6]))
    return v.clamp(-1, 1).numpy().astype(float).tolist()


# ---- games and probe points --------------------------------------------------------------------------------------
def move_users(lines, side: int) -> dict:
    """{Pokemon name: {move ids used}} of ``side`` from player-view ``|move|`` lines."""
    used: dict = {}
    prefix = f"p{side + 1}"
    for line in lines:
        parts = line.split("|")
        if len(parts) > 3 and parts[1] == "move" and parts[2].startswith(prefix):
            used.setdefault(parts[2].split(": ", 1)[1], set()).add(to_id(parts[3]))
    return used


def play_game(players: Players, sleep: SleepMoves, teams, seed, rng, max_turns: int):
    """One self-play game; returns its history (choices per boundary, player-log lengths) and the probe points."""
    battle = numbrion.Battle(seed, teams[0], teams[1], log=True)
    battle.start()
    battles = [battle]
    logs = [[], []]
    history, points = [], []
    spore, hypnosis = (sleep.props[int(sleep.kind[sleep.dex.move_index[m]])] for m in ("spore", "hypnosis"))
    while not battle.ended and battle.turn <= max_turns:
        rows = [s for s in (0, 1) if battle.needs_action(s)]
        lines = {s: battle.drain_player_log(s) for s in (0, 1)}
        for s in (0, 1):
            logs[s].extend(lines[s])
        observations = [dict(battle_id=1, request=battle.request_json(s), log=lines[s]) for s in (0, 1)]
        players.encoder.encode_rows([0, 1], observations, *players.outputs())
        moving = all(json.loads(battle.request_json(s) or "{}").get("active") and
                     not json.loads(battle.request_json(s)).get("forceSwitch") for s in (0, 1))
        if moving and len(rows) == 2:
            arrays = players.buffers.arrays()
            for s in (0, 1):
                o = 1 - s
                request = json.loads(battle.request_json(o))
                used = move_users(logs[o], o)
                for i in (0, 1):  # Y: the opponent's active in slot i
                    mon = request["side"]["pokemon"][i]
                    if not mon.get("active") or mon["condition"].endswith("fnt"):
                        continue
                    name = mon["ident"].split(": ", 1)[1]
                    free = [m for m, move in enumerate(mon["moves"]) if move not in used.get(name, set())]
                    if not free:
                        continue
                    ability = int(arrays["ids"][o, i, 2])
                    for j in (0, 1):  # X: S's active in slot j
                        if all(sleep.effective(arrays["ids"], arrays["floats"], arrays["field"], s, j, props, ability)
                               for props in (spore, hypnosis)):
                            points.append(dict(boundary=len(history), turn=battle.turn, side=s, target=j, user=i,
                                               user_name=name, slot=free[-1]))
        picks = players.act(rows, battles, rng) if rows else {}
        history.append(dict(choices=[picks.get(s) for s in (0, 1)], cut=(len(logs[0]), len(logs[1]))))
        for s in rows:
            battle.choose_action(s, *picks[s])
    return history, points, logs


def replace_move(team: str, name: str, slot: int, move: str) -> str | None:
    sets = team.split("]")
    for k, packed in enumerate(sets):
        fields = packed.split("|")
        if (fields[0] or fields[1]) == name or fields[1] == name:
            moves = fields[4].split(",")
            if slot >= len(moves):
                return None
            moves[slot] = move
            fields[4] = ",".join(moves)
            sets[k] = "|".join(fields)
            return "]".join(sets)
    return None


def replay(teams, seed, history, upto: int, logs, dex):
    """The game replayed to boundary ``upto`` with an incremental encoder; None unless it reproduces ``logs``."""
    battle = numbrion.Battle(seed, teams[0], teams[1], log=True)
    battle.start()
    encoder = numbrion.ObsEncoder(str(dex.path), 2)
    buffers = Buffers(2)
    seen = [[], []]
    outputs = (buffers.ids, buffers.floats, buffers.field, buffers.foe_match, buffers.fainted, buffers.guard,
               buffers.action_features, buffers.token_features)
    for t in range(upto + 1):
        lines = [battle.drain_player_log(s) for s in (0, 1)]
        for s in (0, 1):
            seen[s].extend(lines[s])
        encoder.encode_rows([0, 1], [dict(battle_id=1, request=battle.request_json(s), log=lines[s]) for s in (0, 1)],
                            *outputs)
        if t == upto:
            break
        for s, pick in enumerate(history[t]["choices"]):
            if pick is not None:
                try:
                    battle.choose_action(s, *pick)
                except Exception:
                    return None
    cut = history[upto]["cut"]
    if any(seen[s] != logs[s][:cut[s]] for s in (0, 1)):
        return None
    return battle, encoder, buffers


def probe_position(model, dex, teams, seed, history, logs, point, rng, tries: int):
    """The asleep (A) and awake (B) copies of one probe point at their next boundary, or (None, reason)."""
    s, o, i, j = point["side"], 1 - point["side"], point["user"], point["target"]
    copies = []
    for move in ("spore", "hypnosis"):
        team = replace_move(teams[o], point["user_name"], point["slot"], move)
        if team is None:
            return None, "set not found"
        modified = list(teams)
        modified[o] = team
        made = replay(modified, seed, history, point["boundary"], logs, dex)
        if made is None:
            return None, "replay differs"
        copies.append(made)
    code = point["slot"] * 10 + (3 + j) * 2
    (battle_a, encoder_a, buffers_a), (battle_b, encoder_b, buffers_b) = copies
    for battle in (battle_a, battle_b):
        mask0, joint = battle.legal_mask(o)
        if not (joint.any(1)[code] if i == 0 else joint.any(0)[code]):
            return None, "sleep move not legal"
    # One policy sample for S and for Y's partner, from copy A's views (S's view is the same in both copies).
    players = Players(model, dex, 1, 1.0)
    players.buffers = buffers_a
    picks = players.act([s], [battle_a], rng)
    their = players.act([o], [battle_a], rng, fixed={o: (i, code)})
    for attempt in range(tries):
        turn_seed = [int(x) for x in rng.integers(0, 1 << 16, size=4)]
        trial = []
        for battle in (battle_a, battle_b):
            copy = battle.clone()
            copy.reseed(turn_seed)
            copy.drain_log()
            for side, pick in ((s, picks[s]), (o, their[o])):
                copy.choose_action(side, *pick)
            trial.append((copy, copy.drain_log()))
        (a, log_a), (b, log_b) = trial
        y = f"p{o + 1}{'ab'[i]}"
        status = [line.split("|") for line in log_a if line.startswith("|-status|")]
        slept = [parts[2] for parts in status if len(parts) > 4 and parts[3] == "slp"
                 and parts[4] == "[from] move: Spore" and parts[2].startswith(f"p{s + 1}{'ab'[j]}")]
        missed = [line for line in log_b if line.startswith("|-miss|") and line.split("|")[2].startswith(y)]
        if len(slept) == 1 and missed and missed[0].split("|")[3] == slept[0] and not a.ended and not b.ended:
            return (a, b, encoder_a, encoder_b, slept[0].split(": ", 1)[1], attempt + 1), None
    return None, "no seed where Spore sleeps and Hypnosis misses"


def head_values(heads: dict, encoder, battle, side) -> dict:
    """Each head's value for ``side`` at ``battle``'s boundary, from a copy of ``encoder``'s trackers."""
    b = Buffers(2)
    copy = numbrion.ObsEncoder(str(load_dex().path), 2)
    for row in (0, 1):
        copy.restore(row, encoder.snapshot(row))
    copy.encode_rows([0, 1], [dict(battle_id=1, request=battle.request_json(r), log=battle.drain_player_log(r))
                              for r in (0, 1)],
                     b.ids, b.floats, b.field, b.foe_match, b.fainted, b.guard, b.action_features, b.token_features)
    return {name: values(model, b, [side])[0] for name, model in heads.items()}, copy


def playouts(model, dex, starts, temperature, max_turns, gamma):
    """Policy self-play to the end from each (battle, encoder with rows 0 / 1 at its boundary, side, seed, rng seed).
    Returns per start (result, discounted result) from ``side``'s perspective."""
    n = len(starts)
    players = Players(model, dex, n, temperature)
    battles, sides, rngs = [], [], []
    for k, (battle, encoder, side, seed, rng_seed) in enumerate(starts):
        copy = battle.clone()
        copy.reseed(list(seed))
        for row in (0, 1):
            players.encoder.restore(2 * k + row, encoder.snapshot(row))
        battles.append(copy)
        sides.append(side)
        rngs.append(np.random.default_rng(rng_seed))
    steps = np.zeros(n, dtype=np.int64)
    live = [k for k in range(n) if not battles[k].ended]
    while live:
        rows = [2 * k + s for k in live for s in (0, 1) if battles[k].needs_action(s)]
        players.encode(rows, battles)
        picks = players.act(rows, battles, rngs)  # per battle, its own sampling stream (A and B playouts share it)
        for r in rows:
            battles[r // 2].choose_action(r % 2, *picks[r])
        for k in live:
            steps[k] += 1
        live = [k for k in live if not battles[k].ended and battles[k].turn <= max_turns]
    out = []
    for k, battle in enumerate(battles):
        winner = battle.winner if battle.ended else None
        result = 0.0 if winner is None else 1.0 if winner == sides[k] else -1.0
        out.append((result, result * gamma ** int(steps[k])))
    return out


def mean_se(values) -> tuple[float, float]:
    values = np.asarray(values, dtype=np.float64)
    if not len(values):
        return float("nan"), float("nan")
    se = values.std(ddof=1) / np.sqrt(len(values)) if len(values) > 1 else float("nan")
    return float(values.mean()), float(se)


def main(argv=None) -> dict:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--checkpoint", type=Path, required=True, help="policy for games and playouts; its head too")
    parser.add_argument("--heads", type=Path, nargs="*", default=[], help="more value heads to score")
    parser.add_argument("--pool", type=Path, default=REPO / "data/teams/eval-s43-200.txt")
    parser.add_argument("--positions", type=int, default=200)
    parser.add_argument("--playouts", type=int, default=16, help="per copy (asleep / awake)")
    parser.add_argument("--temperature", type=float, default=1.0, help="policy temperature for games and playouts")
    parser.add_argument("--max-turns", type=int, default=300)
    parser.add_argument("--gamma", type=float, default=0.995)
    parser.add_argument("--tries", type=int, default=16, help="turn seeds tried per position")
    parser.add_argument("--max-games", type=int, default=0, help="stop after this many games (0: 5 x positions)")
    parser.add_argument("--chunk", type=int, default=8, help="positions whose playouts run together")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--threads", type=int, default=2, help="torch threads")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    torch.set_num_threads(args.threads)
    args.out.mkdir(parents=True, exist_ok=True)
    dex = load_dex()
    model = load_model(args.checkpoint, dex)
    heads = {args.checkpoint.stem: model}
    for path in args.heads:
        heads[path.stem if path.stem not in heads else str(path)] = load_model(path, dex)
    sleep = SleepMoves(dex, ("spore", "hypnosis"))
    teams = [line.strip() for line in args.pool.read_text().splitlines() if line.strip()]
    rng = np.random.default_rng(args.seed)
    game_players = Players(model, dex, 1, args.temperature)
    max_games = args.max_games or 5 * args.positions
    skipped: dict = {}
    pending, results = [], []
    started = time.time()
    games = 0
    rows_path = args.out / "positions.jsonl"
    rows_path.write_text("")

    def flush():
        starts = []
        for p in pending:
            for k in range(args.playouts):
                seed, rng_seed = [int(x) for x in rng.integers(0, 1 << 16, size=4)], int(rng.integers(1 << 62))
                starts.append((p["a"], p["enc_a"], p["side"], seed, rng_seed))
                starts.append((p["b"], p["enc_b"], p["side"], seed, rng_seed))
        outcomes = playouts(model, dex, starts, args.temperature, args.max_turns, args.gamma)
        for n, p in enumerate(pending):
            chunk = outcomes[2 * n * args.playouts:2 * (n + 1) * args.playouts]
            asleep, awake = np.array(chunk[0::2]), np.array(chunk[1::2])
            row = {k: v for k, v in p.items() if k not in ("a", "b", "enc_a", "enc_b")}
            row.update(playouts=args.playouts, result_asleep=float(asleep[:, 0].mean()),
                       result_awake=float(awake[:, 0].mean()),
                       empirical_cost=float(awake[:, 0].mean() - asleep[:, 0].mean()),
                       empirical_cost_discounted=float(awake[:, 1].mean() - asleep[:, 1].mean()))
            results.append(row)
            with rows_path.open("a") as stream:
                stream.write(json.dumps(row) + "\n")
        pending.clear()
        print(f"{len(results)} positions, {games} games, {time.time() - started:.0f} s, skipped {skipped}", flush=True)

    while len(results) + len(pending) < args.positions and games < max_games:
        i, j = rng.choice(len(teams), size=2, replace=False)
        pair = (teams[i], teams[j])
        seed = [int(x) for x in rng.integers(0, 1 << 16, size=4)]
        game_players.encoder = numbrion.ObsEncoder(str(dex.path), 2)
        history, points, logs = play_game(game_players, sleep, pair, seed, rng, args.max_turns)
        games += 1
        if not points:
            skipped["no candidate"] = skipped.get("no candidate", 0) + 1
            continue
        point = points[int(rng.integers(len(points)))]
        made, reason = probe_position(model, dex, pair, seed, history, logs, point, rng, args.tries)
        if made is None:
            skipped[reason] = skipped.get(reason, 0) + 1
            continue
        a, b, encoder_a, encoder_b, slept, attempts = made
        side = point["side"]
        values_a, enc_a = head_values(heads, encoder_a, a, side)
        values_b, enc_b = head_values(heads, encoder_b, b, side)  # enc_*: trackers at the next boundary
        pending.append(dict(game=games, turn=point["turn"], side=side, target=slept, user=point["user_name"],
                            seed_attempts=attempts,
                            head_asleep={k: values_a[k] for k in heads}, head_awake={k: values_b[k] for k in heads},
                            head_cost={k: values_b[k] - values_a[k] for k in heads},
                            a=a, b=b, enc_a=enc_a, enc_b=enc_b))
        if len(pending) >= args.chunk:
            flush()
    if pending:
        flush()
    summary = dict(positions=len(results), games=games, playouts=args.playouts, skipped=skipped,
                   seconds=round(time.time() - started, 1), checkpoint=str(args.checkpoint))
    for name in heads:
        cost = [r["head_cost"][name] for r in results]
        summary[f"head_cost[{name}]"] = mean_se(cost)
        summary[f"head_minus_empirical_discounted[{name}]"] = mean_se(
            [r["head_cost"][name] - r["empirical_cost_discounted"] for r in results])
    summary["empirical_cost"] = mean_se([r["empirical_cost"] for r in results])
    summary["empirical_cost_discounted"] = mean_se([r["empirical_cost_discounted"] for r in results])
    (args.out / "summary.json").write_text(json.dumps(summary, indent=1) + "\n")
    print("Summary (mean, standard error): " + json.dumps(summary), flush=True)
    return summary


if __name__ == "__main__":
    main()
