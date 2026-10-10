"""Tests for the learning-setup changes after the first run: adaptive entropy, PFSP opponents, the scripted
heuristic, potential-based KO shaping and the action-usage diagnostics (docs/training/SELFPLAY.md)."""
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest

import numbrion as nb
from training.diagnostics import ActionUsage
from training.dex import load_dex
from training.encoder import I_MOVE0, N_IDS, Encoder
from training.league import HEURISTIC, SELF, SNAPSHOT, SnapshotPool, opponent_kinds, pfsp_weights
from training.ppo import PASS, EntropyController, ko_shaping, sample
from training.scripted import HeuristicAgent, RandomAgent, forced_logprobs

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"


# ---- adaptive entropy coefficient ---------------------------------------------------------------------------------

def test_entropy_target_schedule_and_direction():
    c = EntropyController(0.01, 2.5, 1.5, rate=0.2, low=1e-3, high=0.05)
    assert c.target(0.0) == pytest.approx(2.5) and c.target(0.5) == pytest.approx(2.0)
    assert c.target(1.0) == pytest.approx(1.5) and c.target(3.0) == pytest.approx(1.5)
    before = c.coef
    assert c.update(1.0, 0.0) > before  # entropy below target: stronger bonus
    up = c.coef
    assert c.update(4.0, 0.0) < up  # above target: weaker bonus
    # The error is clipped to one nat, so one update moves the coefficient by at most exp(rate).
    c = EntropyController(0.01, 2.5, 1.5, rate=0.2)
    assert c.update(-100.0, 0.0) == pytest.approx(0.01 * math.exp(0.2))
    assert c.update(float("nan"), 0.0) == pytest.approx(0.01 * math.exp(0.2))  # ignored


def test_entropy_coefficient_stays_in_range_and_resumes():
    c = EntropyController(0.01, 2.5, 1.5, rate=0.2, low=1e-3, high=0.05)
    for _ in range(200):
        c.update(0.0, 0.5)
    assert c.coef == pytest.approx(0.05)
    for _ in range(200):
        c.update(9.0, 0.5)
    assert c.coef == pytest.approx(1e-3)
    c.update(1.0, 0.5)
    other = EntropyController(0.01, 2.5, 1.5, rate=0.2, low=1e-3, high=0.05)
    other.load_state_dict(c.state_dict())
    assert other.coef == pytest.approx(c.coef)


def test_entropy_controller_settles_on_target_in_closed_loop():
    """A toy policy whose entropy relaxes toward 1 + 40 * coef settles near the target (no runaway oscillation)."""
    c = EntropyController(0.01, 2.0, 2.0, rate=0.2, low=1e-3, high=0.05)
    entropy, trace = 4.0, []
    for _ in range(400):
        coef = c.coef
        entropy = 0.7 * entropy + 0.3 * (1.0 + 40 * coef)
        c.update(entropy, 0.5)
        trace.append(entropy)
    assert abs(np.mean(trace[-50:]) - 2.0) < 0.05 and np.ptp(trace[-50:]) < 0.1
    assert c.coef == pytest.approx(0.025, rel=0.1)


# ---- opponents and PFSP --------------------------------------------------------------------------------------------

def test_opponent_kinds_are_spread_over_actors():
    kinds = opponent_kinds(128, 0.3, 0.33)
    assert (kinds != SELF).sum() == 38 and (kinds == HEURISTIC).sum() == 12 and (kinds == SNAPSHOT).sum() == 26
    for chunk in np.array_split(kinds, 6):  # every actor gets self-play, snapshot and heuristic envs
        assert {SELF, SNAPSHOT, HEURISTIC} <= set(chunk.tolist())
    assert (opponent_kinds(16, 0.0, 0.5) == SELF).all()
    assert (opponent_kinds(16, 1.0, 0.0) == SNAPSHOT).all()


def test_pfsp_weights_favour_snapshots_the_learner_loses_to():
    w = pfsp_weights([0.9, 0.5, 0.2], power=2.0, floor=0.1)
    assert w.sum() == pytest.approx(1.0)
    assert w[0] < w[1] < w[2]
    assert w.min() >= 0.1 / 3 - 1e-12  # the uniform floor keeps beaten snapshots in play
    np.testing.assert_allclose(pfsp_weights([1.0, 1.0]), [0.5, 0.5])  # nothing hard: uniform
    np.testing.assert_allclose(pfsp_weights([0.3, 0.3, 0.3]), [1 / 3] * 3)
    assert len(pfsp_weights([])) == 0


def test_snapshot_pool_sampling_ema_eviction_and_resume():
    pool = SnapshotPool(size=4, power=2.0, floor=0.1, alpha=0.02)
    rng = np.random.default_rng(0)
    pool.add({"w": 0}, 0.0)
    assert pool.sample(rng) is None  # the newest snapshot is never an opponent
    for minute in (3.0, 6.0, 9.0):
        pool.add({"w": minute}, minute)
    assert [e["id"] for e in pool.candidates()] == [0, 1, 2]
    pool.record(0, 10, 10.0)  # the learner beats snapshot 0 ...
    assert pool.entries[0]["win"] == pytest.approx(1 - 0.5 * 0.98 ** 10)
    pool.record(2, 50, 5.0)  # ... and mostly loses to snapshot 2
    pool.record(None, 5, 5.0)
    pool.record(1, 0, 0.0)  # ignored
    counts = np.bincount([pool.sample(rng)["id"] for _ in range(4000)], minlength=3)
    np.testing.assert_allclose(counts / 4000, pool.weights(), atol=0.03)
    assert counts[2] > counts[1] > counts[0]
    # Eviction keeps the newest ``size`` snapshots; results for evicted ones are dropped.
    pool.add({"w": 12}, 12.0)
    assert [e["id"] for e in pool.entries] == [1, 2, 3, 4]
    pool.record(0, 10, 0.0)
    restored = SnapshotPool(size=4)
    restored.load_state_dict(pool.state_dict())
    assert [e["id"] for e in restored.entries] == [1, 2, 3, 4] and restored.next_id == 5
    assert restored.entries[1]["win"] == pytest.approx(pool.entries[1]["win"])
    legacy = SnapshotPool(size=2)
    legacy.load_state_dict([{"w": 1}, {"w": 2}, {"w": 3}])  # first-run checkpoints stored a plain list
    assert [e["state"]["w"] for e in legacy.entries] == [2, 3] and legacy.next_id == 3


def test_opponent_results_split_by_kind():
    from train_selfplay import opponent_results

    info = {"opponents": np.array([SELF, SNAPSHOT, HEURISTIC, HEURISTIC], dtype=np.int8)}
    collected = {"games": np.array([2.0, 2.0, 1.0, 3.0]), "score": np.array([1.0, 0.5, 1.0, 2.0]),
                 "battle_turns": np.array([20.0, 18.0, 5.0, 19.0])}
    out = opponent_results(collected, info)
    assert out == {"self_games": 2, "self_turns": 10.0, "pool_games": 2, "pool_win_rate": 0.25, "pool_turns": 9.0,
                   "scripted_games": 4, "scripted_win_rate": 0.75, "scripted_turns": 6.0}


# ---- scripted heuristic --------------------------------------------------------------------------------------------

def test_heuristic_picks_the_strongest_legal_plain_move():
    rng = np.random.default_rng(1)
    rows = 5
    features = np.zeros((rows, 2, 40, 9), dtype=np.float32)
    features[..., 0] = rng.uniform(0.1, 0.5, size=(rows, 2, 40))
    mask = np.zeros((rows, 47), dtype=bool)
    # Row 0: plain codes 0-18 legal; a legal Tera code and an illegal plain code score higher than any legal one.
    mask[0, :20] = True
    features[0, 0, 7, 0] = 5.0  # Tera (odd): never picked
    features[0, 0, 30, 0] = 9.0  # illegal
    features[0, 0, 12, 1] = 10.0  # hits its own side hard: net score negative
    expected0 = int(np.argmax(np.where(np.arange(40) % 2 == 0, features[0, 0, :, 0] - features[0, 0, :, 1], -9)[:20]))
    # Row 1: only switches legal. Row 2: only PASS. Row 3: moves and switches legal but no move scores > 0.
    mask[1, 41:44] = True
    mask[2, PASS] = True
    mask[3, [0, 2, 40, 41]] = True
    features[3, 0, [0, 2], 1] = 1.0
    # Row 4: slot-a features would say code 2, slot b's say code 4 (checked in ``second``).
    mask[4, [2, 4]] = True
    features[4, 0, 2, 0], features[4, 1, 4, 0] = 1.0, 1.0
    agent = HeuristicAgent(rng)
    picks = agent.first({"action_features": features}, np.arange(rows), mask)
    assert picks[0] == expected0 and picks[0] % 2 == 0
    assert picks[1] in (41, 42, 43) and picks[2] == PASS and picks[3] in (0, 2) and picks[4] == 2
    assert all(mask[r, picks[r]] for r in range(rows))
    second = agent.second(np.arange(rows), mask)
    assert second[4] == 4 and all(mask[r, second[r]] for r in range(rows))
    # Forced log-probs make the masked Gumbel sampler return exactly the scripted codes.
    np.testing.assert_array_equal(sample(forced_logprobs(picks), rng), picks)


def test_scripted_players_only_choose_legal_actions():
    rng = np.random.default_rng(2)
    masks = rng.random((500, 47)) < 0.2
    masks[masks.sum(-1) == 0, PASS] = True
    features = rng.normal(size=(500, 2, 40, 9)).astype(np.float32)
    for agent in (RandomAgent(rng), HeuristicAgent(rng)):
        first = agent.first({"action_features": features}, np.arange(500), masks)
        second = agent.second(np.arange(500), masks)
        assert masks[np.arange(500), first].all() and masks[np.arange(500), second].all()
    heuristic = HeuristicAgent(rng).first({"action_features": features}, np.arange(500), masks)
    plain_legal = masks[:, 0:40:2].any(-1)
    assert not ((heuristic < 40) & (heuristic % 2 == 1) & plain_legal).any()  # no Tera while a plain move is legal
    assert not ((heuristic >= 40) & (heuristic < PASS) & plain_legal).any()  # no voluntary switch


def test_heuristic_plays_legal_moves_in_engine_battles():
    env = nb.BatchEnv(4, [str(POOL)], seed=11, threads=1, log=True)
    result = env.reset()
    encoder = Encoder(8)
    agent = HeuristicAgent(np.random.default_rng(3))
    for _ in range(40):
        b = encoder.encode_batch([env.observe(e, s) for e in range(4) for s in (0, 1)])
        active = result["needs_action"].reshape(-1)
        mask0 = result["mask0"].reshape(-1, 47).copy()
        mask0[~active] = False
        mask0[~active, PASS] = True
        rows = np.arange(8)
        a0 = agent.first({"action_features": b.action_features}, rows, mask0)
        mask1 = env.mask_slot1(np.where(active, a0, -1).reshape(4, 2).astype(np.int32)).reshape(-1, 47).copy()
        mask1[~active] = False
        mask1[~active, PASS] = True
        a1 = agent.second(rows, mask1)
        assert mask0[rows, a0].all() and mask1[rows, a1].all()
        actions = np.stack((a0, a1), -1).reshape(4, 2, 2).astype(np.int32)
        actions[~result["needs_action"]] = -1
        result = env.step(actions)
        assert not result["illegal"].any()


# ---- reward shaping ------------------------------------------------------------------------------------------------

def test_ko_shaping_is_potential_based():
    # One row over a battle: (own, foe) KO counts after each step; the battle ends at the last step.
    counts = np.array([[0, 0], [0, 1], [1, 1], [1, 3], [2, 3], [2, 4]], dtype=np.float32)
    rewards = []
    for t in range(1, len(counts)):
        done = np.array([t == len(counts) - 1])
        rewards.append(float(ko_shaping(counts[t - 1:t], counts[t:t + 1], done, 0.02)[0]))
    assert rewards[0] == pytest.approx(0.02) and rewards[1] == pytest.approx(-0.02)
    assert rewards[2] == pytest.approx(0.04)
    assert sum(rewards) == pytest.approx(0.0, abs=1e-7)  # no reward for the KO margin itself
    assert rewards[-1] == pytest.approx(-0.02 * (3 - 2))  # the potential is paid back when the battle ends


# ---- action-usage diagnostics --------------------------------------------------------------------------------------

def code(move_slot: int, target: int, tera: bool = False) -> int:
    """Action code of move slot 0-3 with target code -2..2 (see test_selfplay.test_action_code_layout)."""
    return move_slot * 10 + (target + 2) * 2 + int(tera)


def test_action_usage_shares():
    dex = load_dex()
    m = dex.move_index
    rows = 4
    ids = np.zeros((1, rows, 12, N_IDS), dtype=np.int64)
    moves = [m["protect"], m["fakeout"], m["thunderbolt"], m["swordsdance"]]
    ids[0, :, 0:2, I_MOVE0:I_MOVE0 + 4] = moves
    mask0 = np.zeros((1, rows, 47), dtype=bool)
    mask1 = np.zeros((1, rows, 47), dtype=bool)
    mask0[0, :, :46] = True
    mask1[0, :, :46] = True
    a0 = np.array([[code(0, 0), code(2, 1, True), 41, code(1, 1)]])  # Protect, Tera Thunderbolt, switch, Fake Out
    a1 = np.array([[code(3, 0), code(1, 2), PASS, code(2, 1)]])  # Swords Dance, Fake Out, PASS, Thunderbolt
    mask1[0, 2] = False
    mask1[0, 2, PASS] = True  # row 2: slot b has nothing to do (not a move decision)
    train = np.array([[True, True, True, False]])  # row 3 is an opponent row: ignored
    data = dict(a0=a0, a1=a1, mask0=mask0, mask1=mask1, ids=ids, train=train)
    out = ActionUsage(dex)(data)
    assert out["use_decisions"] == 5
    assert out["use_protect"] == pytest.approx(1 / 5) and out["use_status"] == pytest.approx(1 / 5)
    assert out["use_fake_out"] == pytest.approx(1 / 5) and out["use_switch"] == pytest.approx(1 / 5)
    assert out["use_tera"] == pytest.approx(1 / 5) and out["use_tera_when_legal"] == pytest.approx(1 / 5)
    assert out["choice_slots"] == pytest.approx(5 / 3, abs=1e-3)
    assert ActionUsage(dex)(dict(data, train=np.zeros_like(train))) == {}


# ---- actor integration ---------------------------------------------------------------------------------------------

def test_actors_play_assigned_opponents_and_report_per_env_results():
    """One actor, four envs: self-play, a snapshot and two heuristic opponents on side 2."""
    from training.actors import Actors
    from training.model import Model

    import torch

    torch.manual_seed(0)
    model = Model(load_dex(), width=32, layers=1)
    opponents = np.array([SELF, SNAPSHOT, HEURISTIC, HEURISTIC], dtype=np.int8)
    with Actors(4, 24, [POOL], seed=3, workers=1, model=model) as actors:
        state = {k: v.detach().clone() for k, v in model.state_dict().items()}
        actors.set_weights("learner", state)
        actors.set_weights("opponent", state)
        actors.start(0, opponents, ko_bonus=0.02)
        totals = actors.wait()
        data = {k: v.copy() for k, v in actors.buffers[0].arrays.items()}
    for key in ("games", "score", "clean_games", "clean_score", "battle_turns"):
        assert totals[key].shape == (4,)
    assert totals["games"].sum() == totals["completed"] and (totals["clean_games"] <= totals["games"]).all()
    assert (totals["score"] <= totals["games"]).all() and totals["battle_turns"].sum() == totals["turns"]
    acted = data["mask0"][..., :PASS].any(-1) | data["mask1"][..., :PASS].any(-1)  # rows with a real choice
    # Only the learner's rows are trained: both sides of the self-play env, side 1 elsewhere.
    learner = [0, 1, 2, 4, 6]
    assert (data["train"][:, learner] | ~acted[:, learner]).all() and data["train"][:, learner].any()
    assert not data["train"][:, [3, 5, 7]].any()
    # Heuristic rows play deterministic codes: no Tera while a plain move is legal, no voluntary switch.
    for row in (5, 7):
        for codes, mask in ((data["a0"][:, row], data["mask0"][:, row]), (data["a1"][:, row], data["mask1"][:, row])):
            assert mask[np.arange(len(codes)), codes].all()
            plain = mask[:, 0:40:2].any(-1)
            assert not ((codes < 40) & (codes % 2 == 1) & plain).any()
            assert not ((codes >= 40) & (codes < PASS) & plain).any()
        np.testing.assert_allclose(data["logp"][:, row], 0.0)
    assert np.isfinite(data["reward"]).all() and np.isfinite(data["logp"]).all()


def test_evaluation_temperature():
    from evaluate import tempered

    lp = np.log(np.array([[0.1, 0.6, 0.3] + [1e-30] * 44], dtype=np.float64)).astype(np.float32)
    lp[0, 3:] = -1e9  # masked
    assert tempered(lp, 1.0) is lp
    rng = np.random.default_rng(4)
    greedy = sample(np.repeat(tempered(lp, 0.0), 200, 0), rng)
    assert (greedy == 1).all()
    sharp = np.exp(tempered(lp, 0.5)[0, :3])
    sharp /= sharp.sum()
    assert sharp[1] > 0.6 and np.isclose(sharp.sum(), 1.0)
