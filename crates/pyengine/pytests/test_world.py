"""Fair-information worlds through the Python API (wheel built with ``--features python,search``)."""

import warnings
from pathlib import Path

import numpy as np
import pytest

import numbrion as nb
from conftest import n_battles

pytestmark = pytest.mark.skipif(not nb.SEARCH, reason="wheel built without the search feature")

POOL = Path(__file__).resolve().parents[1] / "testdata" / "pool-400.txt.gz"


@pytest.fixture(scope="module")
def set_pool():
    return nb.SetPool([str(POOL)])


def observe_battle(teams, i, j, seed, viewer, turns, rng):
    """Play random choices; return the real battle and the viewer's observation stream."""
    b = nb.Battle(seed, teams[i], teams[j], names=("Alice", "Bob"), log=True)
    b.start()
    events = [("lines", b.drain_player_log(viewer))]
    while not b.ended and b.turn <= turns:
        for side in (viewer, 1 - viewer):
            if not b.needs_action(side):
                continue
            m0, joint = b.legal_mask(side)
            a0 = int(rng.choice(np.flatnonzero(m0)))
            a1 = int(rng.choice(np.flatnonzero(joint[a0])))
            if side == viewer:
                events.append(("request", b.request_json(viewer)))
                events.append(("action", (a0, a1)))
            b.choose_action(side, a0, a1)
        events.append(("lines", b.drain_player_log(viewer)))
    return b, events


def builder(b, viewer, events, set_pool, seed):
    wb = nb.WorldBuilder(viewer, b.teams[viewer], set_pool, names=b.names, seed=seed)
    for kind, value in events:
        if kind == "lines":
            wb.push_lines(value)
        elif kind == "request":
            wb.push_request(value)
        else:
            wb.push_action(*value)
    return wb


def test_set_pool(set_pool):
    assert len(set_pool) == 2400
    assert 0 < set_pool.distinct < 2400


def test_worlds_match_the_viewer_and_feed_the_grid(teams, set_pool):
    rng = np.random.default_rng(3)
    ex = nb._GRID(2)
    built = 0
    for n in range(n_battles(8)):
        viewer = n % 2
        b, events = observe_battle(teams, 2 * n, 2 * n + 1, (n, 1, 2, 3), viewer, 1 + n % 4, rng)
        if b.ended or not b.needs_action(viewer):
            continue
        wb = builder(b, viewer, events, set_pool, seed=n)
        worlds = wb.worlds(3, seed=100 + n, log=True)
        built += 1
        assert wb.alive >= 1 and wb.stats()["segments"] > 0
        weights, ess = wb.weights
        assert abs(sum(weights) - 1) < 1e-9 and ess >= 1
        state = wb.public_state()
        assert len(state["sides"]) == 2 and state["turn"] == b.turn
        for w in worlds:
            assert w.is_world and not b.is_world
            assert w.turn == b.turn
            assert w.request_json(viewer) == b.request_json(viewer)
            assert w.teams[viewer] != "" and w.clone().is_world
        # Worlds are valid grid roots, and do not trigger the live-root warning.
        m0, joint = worlds[0].legal_mask(viewer)
        a0 = int(np.flatnonzero(m0)[0])
        a1 = int(np.flatnonzero(joint[a0])[0])
        actions = np.full((1, 2, 2), -1, dtype=np.int32)
        actions[0, viewer] = (a0, a1)
        opp = 1 - viewer
        if worlds[0].needs_action(opp):
            m0, joint = worlds[0].legal_mask(opp)
            b0 = int(np.flatnonzero(m0)[0])
            actions[0, opp] = (b0, int(np.flatnonzero(joint[b0])[0]))
        with warnings.catch_warnings():
            warnings.simplefilter("error")
            out = ex.run(worlds[0], actions, seeds=[1, 2, 3, 4])
        assert out["turn"].shape == (1,)
        live = actions.copy()
        if b.needs_action(opp):
            m0, joint = b.legal_mask(opp)
            b0 = int(np.flatnonzero(m0)[0])
            live[0, opp] = (b0, int(np.flatnonzero(joint[b0])[0]))
        with pytest.warns(UserWarning, match="not WorldBuilder worlds"):
            try:
                ex.run(b.clone(), live, seeds=[1, 2, 3, 4])
            except ValueError:
                pass  # the live root may be half-chosen; only the warning matters here
    assert built > 0


def test_same_observations_same_worlds(teams, set_pool):
    rng = np.random.default_rng(5)
    b, events = observe_battle(teams, 0, 1, (9, 9, 9, 9), 0, 3, rng)
    if b.ended or not b.needs_action(0):
        pytest.skip("battle ended early")
    xs = builder(b, 0, events, set_pool, seed=1).worlds(2, seed=2)
    ys = builder(b, 0, events, set_pool, seed=1).worlds(2, seed=2)
    for x, y in zip(xs, ys):
        assert x.teams == y.teams and x.prng_seed == y.prng_seed
        assert x.request_json(1) == y.request_json(1)


def test_tracker_and_action_text(teams):
    rng = np.random.default_rng(1)
    b, events = observe_battle(teams, 4, 5, (1, 1, 1, 1), 1, 2, rng)
    t = nb.RevealTracker()
    for kind, value in events:
        if kind == "lines":
            t.feed(value)
    s = t.state()
    assert s["sides"][0]["team_size"] == 6
    leads = [m for m in s["sides"][0]["mons"] if m["active"] is not None]
    assert leads and all(m["species"] and m["level"] > 0 for m in leads)
    wb = nb.WorldBuilder(1, b.teams[1], nb.SetPool(teams=teams[:20]))
    with pytest.raises(ValueError):
        wb.push_action(99, 0)
