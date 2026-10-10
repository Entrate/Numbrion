"""Search support: restore_from / reset on Battle, and the batched GridExecutor."""

import numpy as np
import pytest

import numbrion as nb
from conftest import n_battles
from test_battle import make, rng_step, signature


def random_pairs(b, side, rng, k):
    """Up to ``k`` random legal (a0, a1) pairs of ``side``; ``[[-1, -1]]`` if it needs no action."""
    if not b.needs_action(side):
        return np.array([[-1, -1]], dtype=np.int32)
    m0, joint = b.legal_mask(side)
    out = []
    for _ in range(k):
        a0 = int(rng.choice(np.flatnonzero(m0)))
        out.append((a0, int(rng.choice(np.flatnonzero(joint[a0])))))
    return np.array(out, dtype=np.int32)


def fresh_step(b, rng):
    """Random legal choices for the sides that act at this boundary. Unlike ``rng_step`` it never lets a side
    choose at the next boundary when the other side's choice already ran the turn (a half-chosen root)."""
    acting = [side for side in (0, 1) if b.needs_action(side)]
    for side in acting:
        m0, joint = b.legal_mask(side)
        a0 = int(rng.choice(np.flatnonzero(m0)))
        b.choose_action(side, a0, int(rng.choice(np.flatnonzero(joint[a0]))))


def roots(teams, rng, count, log=True):
    """Started, unfinished battles at random boundaries (move and replacement requests), logs drained."""
    out = []
    while len(out) < count:
        i, j = rng.integers(len(teams), size=2)
        b = make(teams, int(i), int(j), seed=tuple(int(x) for x in rng.integers(1 << 16, size=4)), log=log)
        for _ in range(int(rng.integers(0, 25))):
            fresh_step(b, rng)
            if b.ended:
                break
        if not b.ended:
            if log:
                b.drain_log(), b.drain_player_log(0), b.drain_player_log(1)
            out.append(b)
    return out


def naive_cell(root, codes, seed, stop):
    """The cell done by hand from Python: clone, reseed, both choices (and default replacements)."""
    c = root.clone()
    if seed is not None:
        c.reseed(tuple(int(x) for x in seed))
    for side in (0, 1):
        if root.needs_action(side):
            c.choose_action(side, int(codes[side][0]), int(codes[side][1]))
    steps = 1
    while stop == "turn" and not c.ended and "switch" in (c.request_kind(0), c.request_kind(1)):
        kinds = (c.request_kind(0), c.request_kind(1))  # one round per boundary
        for side in (0, 1):
            if kinds[side] == "switch":
                c.choose(side, "default")
        steps += 1
    return c, steps


def compare(out, i, c, steps, log):
    kinds = {None: 0, "move": 1, "switch": 2}
    assert out["turn"][i] == c.turn and out["ended"][i] == c.ended and out["steps"][i] == steps
    assert out["winner"][i] == (-1 if c.winner is None else c.winner)
    assert out["requests"][i] == (c.request_json(0), c.request_json(1))
    for side in (0, 1):
        assert out["needs_action"][i, side] == c.needs_action(side)
        assert out["request_kind"][i, side] == kinds[c.request_kind(side)]
        m0, joint = c.legal_mask(side)
        assert (out["mask0"][i, side] == m0).all()
        assert (out["mask1_any"][i, side] == joint.any(axis=0)).all()
        if log:
            assert list(out["logs"][i][side]) == c.drain_player_log(side)
    assert signature(out["leaves"][i]) == signature(c)


@pytest.mark.parametrize("stop", ["boundary", "turn"])
def test_grid_matches_python_loop_and_ignores_threads(teams, stop):
    rng = np.random.default_rng(3 if stop == "boundary" else 4)
    ex1, ex3 = nb.GridExecutor(1), nb.GridExecutor(3)
    assert ex3.threads == 3
    switch_leaves = multi_step = 0
    for t in range(n_battles(12)):
        log = t % 3 != 2
        worlds = roots(teams, rng, 1 + t % 3, log=log)
        cells = [nb.grid_actions(random_pairs(w, 0, rng, 3), random_pairs(w, 1, rng, 3)) for w in worlds]
        actions = np.concatenate(cells)
        world = np.concatenate([np.full(len(c), k) for k, c in enumerate(cells)])
        seeds = [None, rng.integers(1 << 16, size=4), rng.integers(1 << 16, size=(len(actions), 4))][t % 3]
        kw = dict(world=world, seeds=seeds, stop=stop, masks=True, keep_leaves=True)
        out1 = ex1.run(worlds, actions, **kw)
        out3 = ex3.run(worlds, actions, **kw)
        for key in ("ended", "winner", "turn", "steps", "needs_action", "request_kind", "mask0", "mask1_any"):
            assert (out1[key] == out3[key]).all(), key
        assert out1["requests"] == out3["requests"]
        assert ("logs" in out1) == log
        if log:
            assert out1["logs"] == out3["logs"]
        for i in range(len(actions)):
            seed = None if seeds is None else (seeds if seeds.ndim == 1 else seeds[i])
            c, steps = naive_cell(worlds[world[i]], actions[i], seed, stop)
            compare(out1, i, c, steps, log)
            assert signature(out3["leaves"][i]) == signature(c)
            switch_leaves += int(out1["request_kind"][i].max() == 2)
            multi_step += int(out1["steps"][i] > 1)
        # The roots are untouched (still at their boundary, nothing new in their logs).
        for w in worlds:
            assert not w.ended and (not log or w.drain_player_log(0) == [])
    if stop == "turn":
        assert switch_leaves == 0 and multi_step > 0
    else:
        assert multi_step == 0


def test_grid_single_root_defaults_and_errors(teams):
    root = make(teams, 0, 1, log=False)
    ex = nb.GridExecutor(2)
    pairs = [random_pairs(root, s, np.random.default_rng(s), 4) for s in (0, 1)]
    actions = nb.grid_actions(*pairs)
    assert actions.shape == (16, 2, 2) and (actions[5, 0] == pairs[0][1]).all() and (actions[5, 1] == pairs[1][1]).all()
    out = ex.run(root, actions)  # one root, root PRNG, no logs (log=False), requests only
    assert set(out) == {"ended", "winner", "turn", "steps", "needs_action", "request_kind", "requests"}
    assert out["turn"].dtype == np.int32 and out["winner"].dtype == np.int8 and out["needs_action"].shape == (16, 2)
    # Without reseeding the cells replay the root's own future: a plain clone agrees.
    c = root.clone()
    c.choose_action(0, *map(int, actions[0, 0]))
    c.choose_action(1, *map(int, actions[0, 1]))
    assert out["requests"][0] == (c.request_json(0), c.request_json(1))
    bad = actions.copy()
    bad[3, 0] = (nb.PASS, nb.PASS)
    with pytest.raises(ValueError, match="cell 3 side 0.*is not legal"):
        ex.run(root, bad)
    with pytest.raises(ValueError, match="shape"):
        ex.run(root, actions[:, 0])
    with pytest.raises(ValueError, match="seed"):
        ex.run(root, actions, seeds=[1, 2, 3, 70000])
    with pytest.raises(ValueError, match="out of range"):
        ex.run(root, actions, world=np.ones(16, dtype=np.int64))
    with pytest.raises(ValueError, match="log=True"):
        ex.run(root, actions, logs=True)
    with pytest.raises(ValueError, match="started"):
        ex.run(nb.Battle((1, 2, 3, 4), teams[0], teams[1]), actions)
    with pytest.raises(ValueError, match="list of Battles"):
        ex.run("root", actions)
    half = root.clone()
    half.choose_action(0, *map(int, actions[0, 0]))
    with pytest.raises(ValueError, match="already chosen"):
        ex.run(half, actions)
    assert ex.run(root, actions)["requests"] == out["requests"]


def test_restore_from_adopts_any_root(teams):
    rng = np.random.default_rng(11)
    a, b = roots(teams, rng, 2)
    worker = make(teams, 5, 6, seed=(9, 9, 9, 9))
    worker.drain_log()
    rng_step(worker, rng)  # leave unread lines and a different state behind
    worker.restore_from(a)
    assert signature(worker) == signature(a) and worker.teams == a.teams and worker.names == a.names
    assert worker.drain_player_log(0) == [] and worker.drain_log() == []
    worker.restore_from(b)
    assert signature(worker) == signature(b) and worker.teams == b.teams
    with pytest.raises(ValueError):
        worker.restore_from(make(teams, 0, 1, log=False))
    worker.restore_from(worker)


def test_restored_worker_plays_like_the_root(teams):
    rng = np.random.default_rng(12)
    worker = make(teams, 7, 8, seed=(1, 1, 1, 1))
    for root in roots(teams, rng, 4):
        worker.restore_from(root)
        while not root.ended:
            seed = int(rng.integers(1 << 30))
            rng_step(root, np.random.default_rng(seed))
            rng_step(worker, np.random.default_rng(seed))
            assert signature(worker) == signature(root)
            assert worker.drain_player_log(1) == root.drain_player_log(1)


def test_reset_matches_a_fresh_battle(teams):
    b = make(teams, 0, 1)
    rng = np.random.default_rng(5)
    for _ in range(5):
        rng_step(b, rng)
    b.reset((5, 6, 7, 8))
    assert not b.started and b.turn == 0
    b.start()
    fresh = make(teams, 0, 1, seed=(5, 6, 7, 8))
    assert signature(b) == signature(fresh)
    assert b.drain_log() == fresh.drain_log()
    b.reset((7, 7, 7, 7), p1=teams[4], p2=teams[5])
    b.start()
    fresh = make(teams, 4, 5, seed=(7, 7, 7, 7))
    assert signature(b) == signature(fresh) and b.teams == (teams[4], teams[5]) and b.names == ("Alice", "Bob")
    for _ in range(3):
        s = int(rng.integers(1 << 30))
        rng_step(b, np.random.default_rng(s))
        rng_step(fresh, np.random.default_rng(s))
        assert signature(b) == signature(fresh)
    before = signature(b)
    with pytest.raises(ValueError):
        b.reset((1, 2, 3, 4), p1="not a team")
    assert signature(b) == before


def test_determinized_worlds_in_one_grid_call(teams):
    """Worlds made with replace_hidden_set are ordinary roots: one call over all of them equals a per-world loop."""
    rng = np.random.default_rng(21)
    ex = nb.GridExecutor(3)
    for root in roots(teams, rng, n_battles(6)):
        worlds = [root]
        for _ in range(3):
            w = root.clone()
            for i in w.hidden_team_indices(0):
                donor = teams[int(rng.integers(len(teams)))].split("]")[int(rng.integers(6))]
                try:
                    w.replace_hidden_set(0, i, donor)
                except ValueError:
                    pass  # unsupported swap (e.g. BattleStart species): keep the real set
            worlds.append(w)
        cells = nb.grid_actions(random_pairs(root, 0, rng, 3), random_pairs(root, 1, rng, 3))
        actions = np.concatenate([cells] * len(worlds))
        world = np.repeat(np.arange(len(worlds)), len(cells))
        seed = rng.integers(1 << 16, size=4)
        out = ex.run(worlds, actions, world=world, seeds=seed, keep_leaves=True, masks=True)
        for i in range(len(actions)):
            c, steps = naive_cell(worlds[world[i]], actions[i], seed, "boundary")
            compare(out, i, c, steps, True)
