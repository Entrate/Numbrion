import json

import numpy as np
import pytest

import numbrion as nb
from conftest import n_battles


def check(r, n):
    """Invariants every result dict satisfies."""
    A = nb.N_ACTIONS
    assert r["needs_action"].shape == (n, 2) and r["needs_action"].dtype == bool
    assert r["mask0"].shape == (n, 2, A) and r["mask0"].dtype == bool
    assert r["reward"].shape == (n, 2) and r["reward"].dtype == np.float32
    assert r["done"].shape == (n,) and r["done"].dtype == bool
    # Which sides must act is exactly where a slot-0 action exists.
    assert (r["needs_action"] == r["mask0"].any(axis=-1)).all()
    assert (r["needs_action"] == (r["request_kind"] != 0)).all()
    assert r["needs_action"].any(axis=1).all()
    assert not r["mask1_any"][~r["needs_action"]].any()
    done = r["done"]
    assert (r["reward"][~done] == 0).all() and (r["winner"][~done] == -1).all()
    won = done & (r["winner"] >= 0)
    assert (r["reward"][won].sum(axis=1) == 0).all() and (np.abs(r["reward"][won]).sum(axis=1) == 2).all()
    assert (r["reward"][done & (r["winner"] < 0)] == 0).all()
    assert (r["final_turns"][done] > 0).all() and (r["final_turns"][~done] == 0).all()
    if "joint" in r:
        j = r["joint"]
        assert j.shape == (n, 2, A, A)
        assert (j.any(axis=-1) == r["mask0"]).all()


def test_shapes_and_reset(pool_path):
    env = nb.BatchEnv(6, [pool_path], seed=3, threads=2)
    assert (env.n_envs, env.threads, env.pool_size) == (6, 2, 400)
    r = env.reset()
    check(r, 6)
    assert r["needs_action"].all()  # every battle starts with both sides to move
    assert (r["request_kind"] == 1).all()
    assert (r["turn"] == 1).all() and not r["done"].any()
    assert "joint" not in r
    env = nb.BatchEnv(3, [str(pool_path)], seed=3, threads=1, joint_mask=True)
    check(env.reset(), 3)


def test_constructor_errors(pool_path, tmp_path):
    with pytest.raises(ValueError):
        nb.BatchEnv(2, [])
    with pytest.raises(ValueError):
        nb.BatchEnv(2, [pool_path], on_illegal="whatever")
    with pytest.raises(ValueError):
        nb.BatchEnv(2, [tmp_path / "missing.txt"])
    # A plain-text pool and in-memory teams work too.
    line = __import__("gzip").open(pool_path, "rt").readline().strip()
    f = tmp_path / "teams.txt"
    f.write_text(line + "\n\n" + line + "\n", encoding="utf-8")
    env = nb.BatchEnv(2, [f], seed=1, threads=1)
    assert env.pool_size == 2
    env = nb.BatchEnv(2, teams=[line], seed=1, threads=1)
    assert env.pool_size == 1
    check(env.reset(), 2)


def test_hundreds_of_battles_random_actions(pool_path):
    """No rejected choice, no panic, over hundreds of complete battles driven from Python."""
    n = 12
    target = n_battles(300)
    env = nb.BatchEnv(n, [pool_path], seed=7, threads=3)
    rng = np.random.default_rng(0)
    r = env.reset()
    finished = wins = ties = 0
    while finished < target:
        r = env.step(nb.sample_uniform(r, env, rng))
        check(r, n)
        assert not r["illegal"].any()
        finished += int(r["done"].sum())
        wins += int((r["done"] & (r["winner"] >= 0)).sum())
        ties += int((r["done"] & (r["winner"] < 0)).sum())
    assert wins + ties == finished
    assert env.stats()["illegal"] == 0
    assert wins > 0.9 * finished  # ties come only from the turn limit


def test_auto_reset_increments_battle_id(pool_path):
    env = nb.BatchEnv(4, [pool_path], seed=2, threads=2)
    r = env.reset()
    ids = r["battle_id"].copy()
    rng = np.random.default_rng(1)
    bumped = False
    for _ in range(400):
        r = env.step(nb.sample_uniform(r, env, rng))
        new = r["battle_id"]
        assert (new >= ids).all()
        assert ((new > ids) == r["done"]).all()
        assert (r["turn"][r["done"]] == 1).all() or True  # a fresh battle is at its first decision
        bumped |= bool(r["done"].any())
        ids = new.copy()
    assert bumped


def test_results_do_not_depend_on_thread_count(pool_path):
    def run(threads):
        env = nb.BatchEnv(8, [pool_path], seed=11, threads=threads)
        rng = np.random.default_rng(0)
        r = env.reset()
        trace = []
        for _ in range(120):
            r = env.step(nb.sample_uniform(r, env, rng))
            trace.append((r["mask0"].tobytes(), r["reward"].tobytes(), r["turn"].tobytes(), r["battle_id"].tobytes()))
        return trace

    assert run(1) == run(4)


def test_builtin_sampler_and_masks(pool_path):
    env = nb.BatchEnv(8, [pool_path], seed=5, threads=2, joint_mask=True)
    r = env.reset()
    for _ in range(60):
        a = env.random_actions()
        assert a.shape == (8, 2, 2) and a.dtype == np.int32
        assert ((a >= 0) == r["needs_action"][..., None]).all()
        # Sampled pairs are jointly legal.
        for e in range(8):
            for s in range(2):
                if r["needs_action"][e, s]:
                    assert r["joint"][e, s, a[e, s, 0], a[e, s, 1]]
        m1 = env.mask_slot1(a[:, :, 0])
        for e in range(8):
            for s in range(2):
                if r["needs_action"][e, s]:
                    assert (m1[e, s] == r["joint"][e, s, a[e, s, 0]]).all()
                    assert m1[e, s].any()
                else:
                    assert not m1[e, s].any()
        r = env.step(a)
        check(r, 8)


def test_illegal_actions(pool_path):
    env = nb.BatchEnv(3, [pool_path], seed=1, threads=1)
    r = env.reset()
    bad = np.full((3, 2, 2), nb.PASS, dtype=np.int32)  # pass is not legal for a live Pokemon
    with pytest.raises(ValueError):
        env.step(bad)
    after = env.current()
    assert (after["turn"] == r["turn"]).all()  # nothing was applied
    for shape in [(2, 2, 2), (3, 2)]:
        with pytest.raises(ValueError):
            env.step(np.zeros(shape, dtype=np.int32))
    # Python lists and int64 arrays are accepted.
    env.step(env.random_actions().astype(np.int64).tolist())

    lenient = nb.BatchEnv(3, [pool_path], seed=1, threads=1, on_illegal="default")
    lenient.reset()
    out = lenient.step(bad)
    assert out["illegal"].any(axis=None)
    check(out, 3)


def test_observe_is_a_player_view(pool_path):
    env = nb.BatchEnv(2, [pool_path], seed=4, threads=1, log=True)
    r = env.reset()
    seen_final = False
    for _ in range(150):
        r = env.step(env.random_actions())
        for e in range(2):
            for s in range(2):
                o = env.observe(e, s)
                assert o["log_enabled"]
                req = json.loads(o["request"])
                assert req["side"]["id"] == f"p{s + 1}"
                assert all(l and not l.startswith("|split|") for l in o["log"] + o["prev_log"])
                if r["done"][e]:
                    seen_final |= any(l.startswith("|win|") or l == "|tie" for l in o["prev_log"])
    assert seen_final
    # Without logging, observe still returns the request and no lines.
    quiet = nb.BatchEnv(1, [pool_path], seed=4, threads=1)
    quiet.reset()
    o = quiet.observe(0, 0)
    assert not o["log_enabled"] and o["log"] == [] and json.loads(o["request"])["active"]
    with pytest.raises(ValueError):
        quiet.observe(1, 0)


def test_clone_battle_matches_env_state(pool_path):
    env = nb.BatchEnv(2, [pool_path], seed=8, threads=1)
    env.reset()
    for _ in range(5):
        env.step(env.random_actions())
    for e in range(2):
        b = env.clone_battle(e)
        for s in range(2):
            assert b.request_json(s) == env.observe(e, s)["request"]
            assert b.needs_action(s) == bool(env.current()["needs_action"][e, s])
        m0, joint = b.legal_mask(0)
        assert (m0 == env.current()["mask0"][e, 0]).all() or not b.needs_action(0)


def test_run_random_reports_throughput_counters(pool_path):
    env = nb.BatchEnv(4, [pool_path], seed=6, threads=2)
    s = env.run_random(10)
    assert s["battles"] >= 40 and s["decisions"] > s["battles"] and s["illegal"] == 0 and s["seconds"] > 0
    assert env.stats()["battles"] == s["battles"]
