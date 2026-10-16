"""Fused expansion against a reference composed entirely of existing public APIs.

Run with a python,search wheel. NUMBRION_SEARCH2_TEAMS can select a portable team corpus; on the
shared Mac use eval-s43-200.txt read-only. The checked-in pool is the fallback for CI.
"""
from __future__ import annotations

import gzip
from functools import lru_cache
import json
import os
from pathlib import Path
import sys

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import numbrion as nb
from training.protocol import request_mask
from training.search2_types import Expansion

pytestmark = [pytest.mark.skipif(not getattr(nb, "FUSED_EXPAND", False), reason="requires fused expand wheel"),
              pytest.mark.filterwarnings("ignore:numbrion.GridExecutor"),
              pytest.mark.filterwarnings("ignore:numbrion.SearchGrid")]
REPO = Path(__file__).resolve().parents[3]
DEX = REPO / "data/training/dex.json"
POLICY_KEYS = ("ids", "floats", "field", "token_features", "action_features", "guard")


def teams():
    paths = [Path(os.environ["NUMBRION_SEARCH2_TEAMS"])] if "NUMBRION_SEARCH2_TEAMS" in os.environ else [
        REPO / "data/teams/eval-s43-200.txt",
        Path("/Users/aminaliu/.t3/projects/numbrion-mac/data/teams/eval-s43-200.txt"),
        REPO / "crates/pyengine/testdata/pool-400.txt.gz"]
    path = next(p for p in paths if p.exists())
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rt") as f:
        return [line.strip() for line in f if line.strip()]


def tracker_pair(battle):
    return tuple(nb.TrackerState(s).feed(battle.drain_player_log(s)) for s in range(2))


def pairs(battle, side, count, rng):
    if not battle.needs_action(side):
        return np.full((count, 2), -1, np.int32)
    choices = np.argwhere(battle.legal_mask(side)[1])
    return choices[rng.integers(len(choices), size=count)].astype(np.int32)


def cells(battle, count, rng):
    return np.stack([pairs(battle, s, count, rng) for s in range(2)], axis=1)


def play_step(battle, rng):
    # Compute both pairs before committing either side.
    actions = cells(battle, 1, rng)[0]
    for s in range(2):
        if actions[s, 0] >= 0:
            battle.choose_action(s, *map(int, actions[s]))


@lru_cache(maxsize=2)
def positions(count=6):
    """Real random-doubles positions, including the last boundaries and every forced replacement."""
    pool, rng = teams(), np.random.default_rng(781)
    out = []
    for game in range(count):
        b = nb.Battle(tuple(map(int, rng.integers(1 << 16, size=4))), pool[2 * game], pool[2 * game + 1],
                      names=("Alice", "Bob"), log=True)
        b.start()
        trackers = tracker_pair(b)
        history = []
        for step in range(250):
            if b.ended:
                break
            history.append((b.clone(), trackers))
            play_step(b, rng)
            trackers = tuple(trackers[s].feed(b.drain_player_log(s)) for s in range(2))
            b.drain_log()
        picks = {0, min(3, len(history) - 1), max(0, len(history) - 2), len(history) - 1}
        picks.update(i for i, (root, _) in enumerate(history)
                     if any(root.request_kind(s) == "switch" for s in range(2)))
        out.extend(history[i] for i in sorted(picks))
    return out


def reference(grid, executor, roots, actions, trackers, *, world=None, seeds=None, stop="boundary"):
    """Deliberately unfused, public-API implementation of every Expansion field."""
    roots = roots if isinstance(roots, (list, tuple)) else [roots]
    root_pairs = [trackers] if isinstance(trackers[0], nb.TrackerState) else trackers
    raw = executor.run(roots, actions, world=world, seeds=seeds, stop=stop,
                       keep_leaves=True, logs=True, requests=True, allow_live=True)
    n = len(actions)
    result = {key: raw[key] for key in ("ended", "winner", "needs_action", "request_kind", "requests")}
    result.update(leaves=[], trackers=[], policy={}, request_joint=np.zeros((n, 2, 47, 47), bool),
                  engine_joint=np.zeros((n, 2, 47, 47), bool), timing={})
    states, requests, indices = [], [], []
    for i in range(n):
        if raw["ended"][i]:
            result["leaves"].append(None)
            result["trackers"].append(None)
            continue
        leaf = raw["leaves"][i]
        result["leaves"].append(leaf)
        k = 0 if world is None else int(world[i])
        pair = root_pairs[0 if len(root_pairs) == 1 else k]
        fed = tuple(pair[s].feed(raw["logs"][i][s]) for s in range(2))
        result["trackers"].append(fed)
        for s in range(2):
            result["engine_joint"][i, s] = leaf.legal_mask(s)[1]
            if raw["needs_action"][i, s]:
                req = raw["requests"][i][s]
                states.append(fed[s])
                requests.append(req)
                indices.append((i, s))
                result["request_joint"][i, s] = request_mask(req)[1]
    encoded = grid.encode_states(states, requests)
    for key, rows in encoded.items():
        result["policy"][key] = np.zeros((n, 2, *rows.shape[1:]), rows.dtype)
        for row, (i, s) in enumerate(indices):
            result["policy"][key][i, s] = rows[row]
    return result


def identical(a, b):
    assert a.shape == b.shape and a.dtype == b.dtype
    assert a.tobytes() == b.tobytes()


def check(grid, actual, expected, *, check_continuation=False):
    assert set(actual) == set(Expansion.__dataclass_fields__)
    for key in ("ended", "winner", "needs_action", "request_kind", "request_joint", "engine_joint"):
        identical(actual[key], expected[key])
    assert actual["requests"] == expected["requests"]
    assert set(actual["policy"]) == set(POLICY_KEYS)
    for key in POLICY_KEYS:
        identical(actual["policy"][key], expected["policy"][key])
    live_states = [[], []]
    live_requests = []
    for i, ended in enumerate(expected["ended"]):
        if ended:
            assert actual["leaves"][i] is None and actual["trackers"][i] is None
            continue
        a, b = actual["leaves"][i], expected["leaves"][i]
        assert a.turn == b.turn and a.prng_seed == b.prng_seed and a.teams == b.teams
        assert not a.drain_log() and not b.drain_log()
        for s in range(2):
            assert a.request_json(s) == b.request_json(s)
            assert not a.drain_player_log(s) and not b.drain_player_log(s)
            ta, tb = actual["trackers"][i][s], expected["trackers"][i][s]
            assert (ta.side, ta.turn, ta.battle_id) == (tb.side, tb.turn, tb.battle_id)
            live_states[0].append(ta)
            live_states[1].append(tb)
            live_requests.append(a.request_json(s))
        if check_continuation:
            # Retained battles are usable search roots and are independent of subsequent grid reuse.
            rng = np.random.default_rng(i)
            choices = cells(a, 2, rng)
            executor = nb.GridExecutor(1)
            ar = executor.run(a, choices, seeds=[1, 7, 8, 9], logs=True, allow_live=True)
            br = executor.run(b, choices, seeds=[1, 7, 8, 9], logs=True, allow_live=True)
            assert ar["requests"] == br["requests"] and ar["logs"] == br["logs"]
            identical(ar["winner"], br["winner"])
    # Include waiting sides too: their trackers must receive the lines even though policy rows are zero.
    ae = grid.encode_states(live_states[0], live_requests)
    be = grid.encode_states(live_states[1], live_requests)
    for key in POLICY_KEYS:
        identical(ae[key], be[key])


@pytest.mark.parametrize("stop", ["boundary", "turn"])
def test_real_positions_match_reference_bit_for_bit(stop):
    samples = positions()
    roots, trackers = map(list, zip(*samples))
    rng = np.random.default_rng(77)
    actions = np.concatenate([cells(root, 8, rng) for root in roots])
    world = np.repeat(np.arange(len(roots)), 8)
    # Deliberately scramble world order to catch accidental worker/world grouping.
    order = rng.permutation(len(actions))
    actions, world = actions[order], world[order]
    grid1, grid4 = nb.SearchGrid(DEX, 1), nb.SearchGrid(DEX, 4)
    executor = nb.GridExecutor(4)
    counts = dict(cells=0, ended=0, replacements=0, waiting=0)
    for seeds in (None, [1, 7, 11, 19], rng.integers(1 << 16, size=(len(actions), 4))):
        kwargs = dict(world=world, seeds=seeds, stop=stop)
        expected = reference(grid4, executor, roots, actions, trackers, **kwargs)
        for grid in (grid1, grid4):
            actual = grid.expand(roots, actions, trackers, **kwargs, allow_live=True)
            check(grid, actual, expected)
        counts["cells"] += len(actions)
        counts["ended"] += int(expected["ended"].sum())
        counts["replacements"] += int((expected["request_kind"] == 2).any(axis=1).sum())
        counts["waiting"] += int((expected["needs_action"].sum(axis=1) == 1).sum())
    assert counts["cells"] >= 300 and counts["ended"] > 0
    if stop == "boundary":
        assert counts["replacements"] > 0 and counts["waiting"] > 0


def test_hidden_shadow_tag_and_shared_tracker_pair():
    from training_tests.test_search_eval_retry import hidden_shadow_tag_battle
    b = hidden_shadow_tag_battle()
    trackers = tracker_pair(b)
    # Protect everywhere leaves Shadow Tag hidden and the last active slot's request says maybeTrapped.
    actions = np.array([[[14, 4], [14, 4]]], np.int32)
    grid = nb.SearchGrid(DEX, 4)
    expected = reference(grid, nb.GridExecutor(4), [b, b.clone()], actions, trackers, world=[1])
    actual = grid.expand([b, b.clone()], actions, trackers, world=[1], allow_live=True)
    check(grid, actual, expected, check_continuation=True)
    assert np.any(actual["request_joint"] & ~actual["engine_joint"])
    assert json.loads(actual["requests"][0][0])["active"][1].get("maybeTrapped")


def mon(name, moves, ability="Healer", level=80):
    return f"{name}|||{ability}|{moves}||||||{level}|,,,,,Normal"


@pytest.mark.parametrize("kind", ["pivot", "win", "tie", "revival"])
def test_directed_boundaries(kind):
    # Small teams make terminal and replacement conditions deterministic.
    if kind == "pivot":
        ours = "]".join([mon("Scizor", "uturn", "Technician"), mon("Blissey", "calmmind"), mon("Snorlax", "calmmind")])
        theirs = "]".join([mon("Blissey", "calmmind"), mon("Snorlax", "calmmind")])
    elif kind == "win":
        ours = "]".join([mon("Mewtwo", "psychic", level=100)] * 2)
        theirs = "]".join([mon("Raichu", "calmmind", level=1)] * 2)
    elif kind == "tie":
        ours = theirs = "]".join([mon("Raichu", "calmmind", level=1)] * 4)
    else:
        ours = "]".join([mon("Pawmot", "revivalblessing,machpunch", "Natural Cure"),
                          mon("Raichu", "calmmind", level=1), mon("Blissey", "calmmind")])
        theirs = "]".join([mon("Mewtwo", "psychic,calmmind", level=100), mon("Blissey", "calmmind")])
    b = nb.Battle((1, 2, 3, 4), ours, theirs, log=True)
    b.start()
    trackers = tracker_pair(b)
    if kind == "tie":
        # Showdown's hard turn cap produces a genuine tie without altering engine state.
        for _ in range(999):
            b.choose(0, "switch 3, switch 4")
            b.choose(1, "switch 3, switch 4")
            trackers = tuple(trackers[s].feed(b.drain_player_log(s)) for s in range(2))
            b.drain_log()
        assert not b.ended
    if kind == "revival":
        # Let Mewtwo faint Raichu; answer the resulting replacement before using Revival Blessing.
        b.choose(0, "move 2 +2, move 1")
        b.choose(1, "move 1 +2, move 1")
        trackers = tuple(trackers[s].feed(b.drain_player_log(s)) for s in range(2))
        if b.request_kind(0) == "switch":
            b.choose(0, "default")
            trackers = tuple(trackers[s].feed(b.drain_player_log(s)) for s in range(2))
        b.drain_log()
    grid = nb.SearchGrid(DEX, 4)
    if kind == "pivot":
        actions = np.array([[[6, 4], [4, 4]]], np.int32)
    elif kind == "revival":
        actions = np.array([[[4, 4], [14, 4]]], np.int32)
    else:
        actions = cells(b, 1, np.random.default_rng(8))
        # Do not randomly Tera the level-1 Raichu against Psychic.
        if kind == "win":
            actions = np.array([[[6, 8], [4, 4]]], np.int32)
    expected = reference(grid, nb.GridExecutor(4), b, actions, trackers)
    actual = grid.expand(b, actions, trackers, allow_live=True)
    check(grid, actual, expected, check_continuation=kind == "pivot")
    if kind in ("win", "tie"):
        assert actual["ended"][0]
        assert actual["winner"][0] == (0 if kind == "win" else -1)
    else:
        assert 2 in actual["request_kind"][0]
        if kind == "revival":
            assert "reviving" in actual["requests"][0][0]


def test_validation_empty_batch_and_reuse():
    b, trackers = positions(1)[0]
    grid = nb.SearchGrid(DEX, 2)
    actions = cells(b, 4, np.random.default_rng(3))
    for kwargs, error in [({"world": [99] * 4}, "out of range"),
                          ({"seeds": [1, 2, 3]}, "seeds"), ({"stop": "bad"}, "stop")]:
        with pytest.raises(ValueError, match=error):
            grid.expand(b, actions, trackers, allow_live=True, **kwargs)
    for pair in ((trackers[0],), (trackers[1], trackers[0]), [trackers] * 2):
        with pytest.raises(ValueError, match="trackers"):
            grid.expand(b, actions, pair, allow_live=True)
    bad = actions.copy()
    bad[2, 0] = (-1, -1)
    with pytest.raises(ValueError, match="cell 2 side 0"):
        grid.expand(b, bad, trackers, allow_live=True)
    quiet = nb.Battle((1, 2, 3, 4), *b.teams, log=False)
    quiet.start()
    with pytest.raises(ValueError, match="log=True"):
        grid.expand(quiet, cells(quiet, 1, np.random.default_rng(0)), trackers, allow_live=True)
    for choices in (actions[:0], actions):
        actual = grid.expand(b, choices, trackers, allow_live=True)
        expected = reference(grid, nb.GridExecutor(2), b, choices, trackers)
        check(grid, actual, expected, check_continuation=True)
