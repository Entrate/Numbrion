"""SearchGrid (fused leaf encoding) against the slow path: GridExecutor logs/requests encoded by an ObsEncoder."""

import warnings
from pathlib import Path

import numpy as np
import pytest

import numbrion as nb
from conftest import n_battles
from test_search import fresh_step, random_pairs

pytestmark = pytest.mark.filterwarnings("ignore:numbrion.GridExecutor", "ignore:numbrion.SearchGrid")

DEX = str(Path(__file__).resolve().parents[3] / "data" / "training" / "dex.json")
L = nb.ObsEncoder.layout()
ROW_KEYS = ("ids", "floats", "field", "token_features", "action_features", "guard")


def buffers(rows):
    return dict(
        ids=np.zeros((rows, L["N_TOKENS"], L["N_IDS"]), np.int64),
        floats=np.zeros((rows, L["N_TOKENS"], L["N_POKEMON_FLOATS"]), np.float32),
        field=np.zeros((rows, L["N_FIELD_FLOATS"]), np.float32),
        foe_match=np.zeros((rows, 6), np.int64),
        fainted=np.zeros((rows, 2), np.float32),
        guard=np.zeros((rows, 2, 4, 2), bool),
        action_features=np.zeros((rows, 2, L["N_CODES"], L["N_ACTION_FEATURES"]), np.float32),
        token_features=np.zeros((rows, L["N_TOKENS"], L["N_TOKEN_FEATURES"]), np.float32),
    )


ORDER = ("ids", "floats", "field", "foe_match", "fainted", "guard", "action_features", "token_features")


def encode_batch(enc, observations):
    b = buffers(enc.rows)
    enc.encode_batch(observations, *(b[k] for k in ORDER))
    return b


def same(a, b):
    """Bit-identical arrays (float32 compared through their bit patterns, so -0.0 and NaN count too)."""
    a, b = np.asarray(a), np.asarray(b)
    if a.dtype == np.float32:
        a, b = a.view(np.uint32), b.view(np.uint32)
    return a.shape == b.shape and a.dtype == b.dtype and np.array_equal(a, b)


def obs(request, lines, battle_id=0):
    return {"battle_id": battle_id, "request": request, "log": list(lines)}


def play_boundaries(teams, rng, i, j, log=True):
    """A random battle played to the end (both sides answer each boundary). Returns every boundary as
    (battle copy, observation history up to it); the history holds both sides' (request, lines) per boundary."""
    b = nb.Battle(tuple(int(x) for x in rng.integers(1 << 16, size=4)), teams[i], teams[j], names=("Alice", "Bob"), log=log)
    b.start()
    history, out = [], []
    while True:
        history.append([obs(b.request_json(s), b.drain_player_log(s)) for s in (0, 1)])
        b.drain_log()
        if b.ended:
            return out
        out.append((b.clone(), list(history)))
        fresh_step(b, rng)


def replayed(history):
    """A fresh 2-row ObsEncoder fed a whole observation history."""
    enc = nb.ObsEncoder(DEX, 2)
    for pair in history:
        encode_batch(enc, pair)
    return enc


def expected_eval_side(needs, prefer):
    for side in (prefer, 1 - prefer):
        if needs[side]:
            return side
    return -1


def check_cell(out, i, ref, prefer, action_features=True, guard=True):
    """Cell i of a SearchGrid result against `ref`, the encode_batch buffers of the leaf (row = side)."""
    e = int(out["eval_side"][i])
    assert e == expected_eval_side(out["needs_action"][i], prefer)
    if e < 0:
        assert out["ended"][i]
        for k in ("ids", "floats", "field", "token_features", "partner_ids", "partner_floats"):
            assert not out[k][i].any(), k
        return e
    keys = ["ids", "floats", "field", "token_features"] + ["action_features"] * action_features + ["guard"] * guard
    for k in keys:
        assert same(out[k][i], ref[k][e]), (i, k)
    p = 1 - e
    assert same(out["partner_ids"][i], ref["ids"][p, 0:6]), (i, "partner_ids")
    assert same(out["partner_floats"][i], ref["floats"][p, 0:6]), (i, "partner_floats")
    return e


def test_fused_leaves_match_encode_batch_on_any_thread_count(teams):
    rng = np.random.default_rng(11)
    grid1, grid3 = nb.SearchGrid(DEX, 1), nb.SearchGrid(DEX, 3)
    ex = nb.GridExecutor(2)
    counts = dict(cells=0, ended=0, only0=0, only1=0, other_side=0, switch_roots=0)
    for t in range(n_battles(10)):
        boundaries = play_boundaries(teams, rng, 2 * t, 2 * t + 1)
        if not boundaries:
            continue
        # A random boundary, and late ones (cells that can end the battle).
        picks = sorted({int(rng.integers(len(boundaries))), len(boundaries) - 1, max(0, len(boundaries) - 2)})
        roots, pairs, cells, world = [], [], [], []
        for k, at in enumerate(picks):
            root, history = boundaries[at]
            enc = replayed(history)
            roots.append(root)
            pairs.append((enc.snapshot(0), enc.snapshot(1)))
            c = nb.grid_actions(random_pairs(root, 0, rng, 3), random_pairs(root, 1, rng, 3))
            cells.append(c)
            world.append(np.full(len(c), k))
            counts["switch_roots"] += int("switch" in (root.request_kind(0), root.request_kind(1)))
        actions, world = np.concatenate(cells), np.concatenate(world)
        seeds = [None, rng.integers(1 << 16, size=4), rng.integers(1 << 16, size=(len(actions), 4))][t % 3]
        prefer = t % 2
        kw = dict(world=world, seeds=seeds, masks=True, action_features=True, guard=True)
        out = grid1.run(roots, actions, pairs, prefer, allow_live=True, **kw)
        out3 = grid3.run(roots, actions, pairs, prefer, allow_live=True, **kw)
        assert set(out) == set(out3)
        for key in out:
            assert same(out[key], out3[key]), key
        slow = ex.run(roots, actions, world=world, seeds=seeds, masks=True, allow_live=True)
        for key in ("ended", "winner", "turn", "steps", "needs_action", "request_kind", "mask0", "mask1_any"):
            assert same(out[key], slow[key]), key
        for i in range(len(actions)):
            history = boundaries[picks[world[i]]][1]
            leaf = [obs(slow["requests"][i][s], slow["logs"][i][s]) for s in (0, 1)]
            # Reference: a fresh encoder fed the whole history, then the leaf observations.
            ref = encode_batch(replayed(history), leaf)
            e = check_cell(out, i, ref, prefer)
            # The snapshots restored into another encoder give the same leaf.
            enc = nb.ObsEncoder(DEX, 2)
            enc.restore(0, pairs[world[i]][0])
            enc.restore(1, pairs[world[i]][1])
            assert check_cell(out, i, encode_batch(enc, leaf), prefer) == e
            counts["cells"] += 1
            counts["ended"] += e < 0
            needs = tuple(bool(x) for x in out["needs_action"][i])
            counts["only0"] += needs == (True, False)
            counts["only1"] += needs == (False, True)
            counts["other_side"] += e == 1 - prefer
        # The roots are untouched.
        for r in roots:
            assert not r.ended and r.drain_player_log(0) == []
    assert counts["cells"] > 0 and counts["ended"] > 0, counts
    assert counts["only0"] > 0 and counts["only1"] > 0 and counts["other_side"] > 0, counts
    assert counts["switch_roots"] > 0, counts


def test_eval_side_prefers_the_searching_side(teams):
    rng = np.random.default_rng(5)
    grid = nb.SearchGrid(DEX, 2)
    root, history = play_boundaries(teams, rng, 4, 5)[2]
    enc = replayed(history)
    pair = (enc.snapshot(0), enc.snapshot(1))
    actions = nb.grid_actions(random_pairs(root, 0, rng, 2), random_pairs(root, 1, rng, 2))
    for prefer in (0, 1):
        out = grid.run(root, actions, pair, prefer, seeds=[1, 2, 3, 4], allow_live=True)
        for i in range(len(actions)):
            assert out["eval_side"][i] == expected_eval_side(out["needs_action"][i], prefer)
    assert out["eval_side"].dtype == np.int8 and out["partner_ids"].shape == (len(actions), 6, L["N_IDS"])
    assert "action_features" not in out and "guard" not in out and "mask0" not in out


def test_encode_states_matches_encode_batch(teams):
    rng = np.random.default_rng(8)
    grid = nb.SearchGrid(DEX, 2)
    boundaries = play_boundaries(teams, rng, 6, 7)
    states, requests, lines, refs = [], [], [], []
    for root, history in boundaries[1::3]:
        enc = replayed(history[:-1])
        last = history[-1]
        for s in (0, 1):
            # With the last boundary's lines, and with none (the tracker as it is).
            for fed in (last[s]["log"], None):
                states.append(enc.snapshot(s))
                requests.append(root.request_json(s))
                lines.append(fed)
                ref = nb.ObsEncoder(DEX, 2)
                ref.restore(s, enc.snapshot(s))
                pair = [obs(None, []), obs(None, [])]
                pair[s] = obs(root.request_json(s), fed or [])
                refs.append((s, encode_batch(ref, pair)))
    out = grid.encode_states(states, requests, lines)
    assert out["ids"].shape == (len(states), 12, L["N_IDS"])
    for i, (s, ref) in enumerate(refs):
        for k in ROW_KEYS:
            assert same(out[k][i], ref[k][s]), (i, k)
    lean = grid.encode_states(states, requests, lines, action_features=False, guard=False)
    assert set(lean) == {"ids", "floats", "field", "token_features"}
    for k in lean:
        assert same(lean[k], out[k]), k
    # No request and no lines: the tracker's view with an empty request, as encode_row gives it.
    empty = grid.encode_states(states[:1], [None])
    ref = nb.ObsEncoder(DEX, 2)
    ref.restore(refs[0][0], states[0])
    pair = [obs(None, []), obs(None, [])]
    want = encode_batch(ref, pair)
    for k in ROW_KEYS:
        assert same(empty[k][0], want[k][refs[0][0]]), k


def test_snapshot_restore_and_tracker_state(teams):
    rng = np.random.default_rng(2)
    boundaries = play_boundaries(teams, rng, 8, 9)
    _, history = boundaries[4]
    enc = replayed(history[:3])
    s0, s1 = enc.snapshot(0), enc.snapshot(1)
    assert (s0.side, s1.side, s0.battle_id, s1.battle_id) == (0, 1, 0, 0)
    for pair in history[3:]:
        encode_batch(enc, pair)
    assert s0.turn < enc.snapshot(0).turn  # a snapshot is a copy
    # Restoring the snapshots and replaying the rest reproduces the encoder.
    other = nb.ObsEncoder(DEX, 2)
    other.restore(0, s0)
    other.restore(1, s1)
    for pair in history[3:]:
        encode_batch(other, pair)
    last = [obs(p["request"], []) for p in history[-1]]
    a, b = encode_batch(other, last), encode_batch(enc, last)
    for k in ORDER:
        assert same(a[k], b[k]), k
    # TrackerState.feed is a fed copy.
    fed = nb.TrackerState(0).feed([line for pair in history[:3] for line in pair[0]["log"]])
    assert fed.turn == s0.turn and nb.TrackerState(1).turn == 0
    with pytest.raises(ValueError, match="side-1"):
        other.restore(0, s1)
    with pytest.raises(ValueError, match="row"):
        other.snapshot(2)
    with pytest.raises(ValueError):
        nb.TrackerState(2)


def test_input_validation(teams):
    rng = np.random.default_rng(4)
    root, history = play_boundaries(teams, rng, 10, 11)[1]
    enc = replayed(history)
    pair = (enc.snapshot(0), enc.snapshot(1))
    grid = nb.SearchGrid(DEX, 2)
    actions = nb.grid_actions(random_pairs(root, 0, rng, 2), random_pairs(root, 1, rng, 2))
    bad = actions.copy()
    bad[2, 0] = (nb.PASS, nb.PASS)
    with pytest.raises(ValueError, match="cell 2 side 0.*is not legal"):
        grid.run(root, bad, pair, 0, allow_live=True)
    with pytest.raises(ValueError, match="trackers"):
        grid.run(root, actions, (pair[0],), 0, allow_live=True)
    with pytest.raises(ValueError, match="trackers"):
        grid.run(root, actions, [pair, pair], 0, allow_live=True)  # two pairs for one root
    with pytest.raises(ValueError, match=r"trackers\[0\]\[0\] tracks side 1"):
        grid.run(root, actions, (pair[1], pair[0]), 0, allow_live=True)
    with pytest.raises(ValueError, match="eval_side"):
        grid.run(root, actions, pair, 2, allow_live=True)
    with pytest.raises(ValueError, match="shape"):
        grid.run(root, actions[:, 0], pair, 0, allow_live=True)
    with pytest.raises(ValueError, match="out of range"):
        grid.run(root, actions, pair, 0, world=np.ones(len(actions), dtype=np.int64), allow_live=True)
    quiet = nb.Battle((1, 2, 3, 4), teams[0], teams[1], log=False)
    quiet.start()
    with pytest.raises(ValueError, match="log=True"):
        grid.run(quiet, nb.grid_actions(random_pairs(quiet, 0, rng, 1), random_pairs(quiet, 1, rng, 1)), pair, 0,
                 allow_live=True)
    with pytest.raises(ValueError, match="requests"):
        grid.encode_states([pair[0]], [])
    with pytest.raises(ValueError, match="lines"):
        grid.encode_states([pair[0]], [None], [[], []])
    with pytest.raises(ValueError, match="state 0"):
        grid.encode_states([pair[0]], ["{not json"])
    # Nothing ran: a valid call still works and the grid is reusable after errors.
    assert grid.run(root, actions, pair, 0, allow_live=True)["ids"].shape[0] == len(actions)
    if nb.SEARCH:
        with pytest.warns(UserWarning, match="SearchGrid.run: some roots are not WorldBuilder worlds"):
            grid.run(root, actions, pair, 0)


@pytest.mark.skipif(not nb.SEARCH, reason="wheel built without the search feature")
def test_world_roots_with_the_bots_tracker_pair(teams):
    from test_world import observe_battle

    set_pool = nb.SetPool([str(Path(__file__).resolve().parents[1] / "testdata" / "pool-400.txt.gz")])
    rng = np.random.default_rng(13)
    grid = nb.SearchGrid(DEX, 3)
    ex = nb._GRID(2)
    checked = 0
    for n in range(n_battles(6)):
        viewer = n % 2
        b, events = observe_battle(teams, 40 + 2 * n, 41 + 2 * n, (n, 7, 7, 7), viewer, 2 + n % 4, rng)
        if b.ended or not b.needs_action(viewer):
            continue
        # The bot's encoder: its own row and the opponent row are both fed the bot's lines.
        enc = nb.ObsEncoder(DEX, 2)
        wb = nb.WorldBuilder(viewer, b.teams[viewer], set_pool, names=b.names, seed=n)
        request = None
        for kind, value in events:
            if kind == "lines":
                wb.push_lines(value)
                rows = [None, None]
                rows[viewer] = obs(request, value)
                rows[1 - viewer] = obs(None, value)
                encode_batch(enc, rows)
            elif kind == "request":
                wb.push_request(value)
                request = value
            else:
                wb.push_action(*value)
        try:
            worlds = wb.worlds(3, seed=50 + n, log=True)
        except RuntimeError:
            continue  # no world reproduced the history (a few percent of positions): the bot falls back

        pair = (enc.snapshot(0), enc.snapshot(1))
        opp = 1 - viewer
        cells, world = [], []
        for k, w in enumerate(worlds):
            c = nb.grid_actions(random_pairs(w, 0, rng, 3), random_pairs(w, 1, rng, 3))
            cells.append(c)
            world.append(np.full(len(c), k))
        actions, world = np.concatenate(cells), np.concatenate(world)
        seeds = rng.integers(1 << 16, size=4)
        with warnings.catch_warnings():
            warnings.simplefilter("error", UserWarning)  # worlds are fair-information roots: no warning
            out = grid.run(worlds, actions, pair, viewer, world=world, seeds=seeds, action_features=True, guard=True)
        slow = ex.run(worlds, actions, world=world, seeds=seeds)
        for i in range(len(actions)):
            ref = nb.ObsEncoder(DEX, 2)
            ref.restore(0, pair[0])
            ref.restore(1, pair[1])
            leaf = [obs(slow["requests"][i][s], slow["logs"][i][s]) for s in (0, 1)]
            check_cell(out, i, encode_batch(ref, leaf), viewer)
            checked += 1
        # (a) The opponent's root observation in each world: opponent row + the world's request, no lines.
        states = grid.encode_states([pair[opp]] * len(worlds), [w.request_json(opp) for w in worlds])
        for k, w in enumerate(worlds):
            ref = nb.ObsEncoder(DEX, 2)
            ref.restore(opp, pair[opp])
            rows = [obs(None, []), obs(None, [])]
            rows[opp] = obs(w.request_json(opp), [])
            want = encode_batch(ref, rows)
            for key in ROW_KEYS:
                assert same(states[key][k], want[key][opp]), (k, key)
    assert checked > 0
