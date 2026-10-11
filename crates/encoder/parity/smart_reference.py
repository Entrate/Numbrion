"""Request-derived masks and deterministic Python smart decisions for the Rust parity runner.

Usage: python smart_reference.py ARRAY_DIR OBS.jsonl OUT_DIR [--debug BOUNDARY ROW]
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[1] / "pyengine"))

import numpy as np
import torch
import arrays
from training import smart
from training.dex import TARGETS, to_id
from training.encoder import PF


def slot_codes(request, floats, slot, dex):
    """Plausible legality, sufficient to feed identical masks to both implementations."""
    team = (request.get("side") or {}).get("pokemon") or []
    active = request.get("active") or []
    forced = request.get("forceSwitch") or []
    if request.get("wait") or not request:
        return [46]
    is_forced = bool(forced[slot]) if slot < len(forced) else False
    if forced and not is_forced:
        return [46]
    switches = []
    for pokemon in team:
        condition = pokemon.get("condition") or ""
        living = "fnt" not in condition and condition.split(" ")[0].split("/")[0] != "0"
        if living and not pokemon.get("active"):
            # Requests use original team order; encoder tokens put the actives first.
            ident = pokemon.get("ident")
            bench = [p for p in team if not p.get("active")]
            token = 2 + next(k for k, p in enumerate(bench) if p.get("ident") == ident)
            if token < 6:
                switches.append(40 + token)
    if is_forced:
        return switches or [46]
    if slot >= len(active) or not active[slot]:
        return [46]
    a = active[slot]
    if floats[slot, PF["fainted"]] > 0.5 or floats[slot, PF["present"]] < 0.5:
        return [46]
    codes = []
    for m, move in enumerate((a.get("moves") or [])[:4]):
        if move.get("disabled") or move.get("pp", 1) <= 0:
            continue
        target = move.get("target")
        if not target:
            mid = dex.move_index.get(to_id(move.get("id") or move.get("move") or ""), 0)
            target = TARGETS[dex.move_target[mid]]
        ally = 0 if slot == 0 else 1
        if target in ("normal", "any"):
            targets = (ally, 3, 4)
        elif target == "adjacentFoe":
            targets = (3, 4)
        elif target == "adjacentAlly":
            targets = (ally,)
        elif target == "adjacentAllyOrSelf":
            targets = (0, 1)
        else:
            targets = (2,)
        for tc in targets:
            codes.append(m * 10 + tc * 2)
            if a.get("canTerastallize"):
                codes.append(m * 10 + tc * 2 + 1)
    if not a.get("trapped") and floats[slot, PF["trapped"]] < 0.5:
        codes.extend(switches)
    return codes or [46]


def debug(agent, mask0, boundary, row):
    c = agent.ctx
    X = agent._candidates(c, row, 0, np.flatnonzero(mask0[row]))
    Y = agent._candidates(c, row, 1, agent._estimate_codes(c, row, 1))
    values = agent._pair_values(c, row, X, Y)
    print("DEBUG", boundary, row, "codes", X["code"].tolist(), Y["code"].tolist())
    for key in ("hp", "q", "foe_protect", "foe_value", "matchup", "protect_share"):
        print(key, c[key][row].tolist())
    for s in (0, 1):
        for code in range(47):
            print("effect", s, code, {k: v[row, s, code].tolist() for k, v in c["e"].items()})
    print("pairs", values.tolist())
    order = np.argsort(-values.ravel(), kind="stable")[:10]
    for index in order:
        a, b = np.unravel_index(index, values.shape)
        print("top", int(X["code"][a]), int(Y["code"][b]), repr(float(values[a, b])))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("array_dir", type=Path)
    parser.add_argument("observations", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--debug", type=int, nargs=2)
    args = parser.parse_args()
    torch.set_num_threads(1)
    smart.NOISE = smart.PROTECT_SKIP = 0
    smart.PROTECT_SHARE = (0.7, 0.7)
    agent = smart.SmartHeuristicAgent(np.random.default_rng(0), temperature=0)
    obs = arrays.load(args.array_dir)
    boundaries, rows = obs["ids"].shape[:2]
    mask0 = np.zeros((boundaries, rows, 47), dtype=bool)
    mask1 = np.zeros((boundaries, rows, 47, 47), dtype=bool)
    records = args.observations.read_text().splitlines()
    assert len(records) == boundaries
    for t, line in enumerate(records):
        requests = json.loads(line)["observations"]
        for r, observation in enumerate(requests):
            request = json.loads(observation["request"]) if observation["request"] else {}
            codes0 = slot_codes(request, obs["floats"][t, r], 0, agent.dex)
            codes1 = slot_codes(request, obs["floats"][t, r], 1, agent.dex)
            mask0[t, r, codes0] = True
            for a0 in range(47):
                remaining = [code for code in codes1 if not (a0 < 40 and a0 % 2 and code < 40 and code % 2)
                             and not (40 <= a0 < 46 and code == a0)]
                mask1[t, r, a0, remaining or [46]] = True
    a0 = np.empty((boundaries, rows), np.int32)
    a1 = np.empty_like(a0)
    indices = np.arange(rows)
    elapsed = 0.0
    for t in range(boundaries):
        arrays_t = {k: obs[k][t] for k in smart.OBS_KEYS}
        start = time.perf_counter()
        a0[t] = agent.first(arrays_t, indices, mask0[t])
        a1[t] = agent.second(indices, mask1[t, indices, a0[t]])
        elapsed += time.perf_counter() - start
        if args.debug and args.debug[0] == t:
            debug(agent, mask0[t], t, args.debug[1])
        if t % 50 == 0:
            print(f"boundary {t}/{boundaries}", flush=True)
    arrays.save(args.out, {**{k: obs[k] for k in smart.OBS_KEYS}, "mask0": mask0, "mask1_given_a0": mask1, "a0": a0, "a1": a1})
    print(f"Python first+second: {elapsed * 1e6 / boundaries / rows:.3f} us/row ({boundaries * rows} rows, {elapsed:.3f}s)")


if __name__ == "__main__":
    main()
