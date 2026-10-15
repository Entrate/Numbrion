"""New-format labels from the actors' shallow search (training.search_opponent) and shallow_labels.py: partner rows,
outcome / remaining_steps against GAE, the separate value model, the unchanged default path, and shards that
distill_train.py trains on and scores."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

import numbrion as nb
import distill_train as trainer
import shallow_labels
from train_selfplay import partner_view
from training.dex import load_dex
from training.distill import DistillData, DistillStream
from training.encoder import Encoder
from training.guards import MoveGuards
from training.model import Model
from training.ppo import gae
from training.search_opponent import OpponentSearch, SearchOpponent, load_bench_sets

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"
OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")
BENCH = load_bench_sets([POOL])


class Capture:
    """A LabelWriter stand-in that keeps the released rows in order."""

    def __init__(self):
        self.rows, self.written = [], 0

    def add(self, row, minutes, teacher="selfplay-search"):
        self.rows.append((row, minutes, teacher))

    def flush(self):
        self.written = len(self.rows)


def play(opponent, model, steps, count=6, seed=11, sides=(1,)):
    """``sides`` searched, the others random; returns per label (row object, env, step, encoder buffers at the
    decision, side), the rewards [steps, count, 2] and done flags [steps, count], and whether waits happened."""
    env = nb.BatchEnv(count, [str(POOL)], seed=seed, threads=1, log=True)
    encoder = Encoder(2 * count)
    result = env.reset()
    encoder.encode_env(env)
    b = encoder.buffers
    records, rewards, dones, choices_log, waits = [], [], [], [], False
    for t in range(steps):
        needs = result["needs_action"]
        waits |= bool((needs.sum(1) == 1).any())
        snapshot = b.copy()
        actions = env.random_actions(0.1, 0.1)
        choices_log.append({})
        with torch.no_grad():
            x = {k: torch.from_numpy(np.ascontiguousarray(getattr(b, k))) for k in OBS_KEYS}
            state = model.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
        for side in sides:
            envs = np.flatnonzero(needs[:, side])
            before = {int(e): len(opponent.pending.get(int(e), [])) for e in envs}
            choices = (opponent.act(model, env, encoder, state, {**b.arrays(), "guard": b.guard}, envs, needs, 1.5,
                                    "teacher.pt", side=side) if len(envs) else {})
            choices_log[-1].update({(e, side): pair for e, pair in choices.items()})
            for e in envs:
                pending = opponent.pending.get(int(e), [])
                if len(pending) > before[int(e)]:
                    assert pending[-1][4] == side
                    records.append((pending[-1][0], int(e), t, snapshot, side))
            for e, pair in choices.items():
                actions[e, side] = pair
        actions[~needs] = -1
        result = env.step(actions)
        assert not result["illegal"].any()
        rewards.append(result["reward"].copy())
        dones.append(result["done"].copy())
        opponent.end_step(result["done"], result["winner"], result["aborted"])
        encoder.encode_env(env)
    return records, np.asarray(rewards, np.float32), np.asarray(dones), choices_log, waits


def searcher(value_model=None, seed=3):
    return SearchOpponent(str(load_dex().path), MoveGuards(), OpponentSearch(iterations=50), seed=seed,
                          labels=Capture(), bench_sets=BENCH, value_model=value_model)


@pytest.mark.parametrize("sides", [(1,), (0, 1)])
def test_partner_rows_outcomes_and_steps_match_the_actor_rows_and_gae(sides):
    torch.manual_seed(0)
    model = Model(load_dex(), 32, 1).eval()
    opponent = searcher()
    steps, count = 120 if len(sides) == 1 else 80, 6
    records, rewards, dones, _, waits = play(opponent, model, steps, count, sides=sides)
    pending_at_end = {id(entry[0]) for rows in opponent.pending.values() for entry in rows}
    opponent.close()
    assert waits  # forced switches / waits are boundaries too
    released = [row for row, _, _ in opponent.labels.rows]
    assert len(released) == len(records) == opponent.released
    assert all(teacher == "teacher.pt" and minutes == 1.5 for _, minutes, teacher in opponent.labels.rows)
    rewards = rewards.reshape(steps, 2 * count)  # actor rows 2e + side
    rows_done = np.repeat(dones, 2, axis=1)
    returns = gae(rewards, np.zeros_like(rewards), rows_done, np.zeros(2 * count, np.float32), .995, 1.0)[1]
    assert {side for *_, side in records} == set(sides)
    finished = capped = 0
    for row, e, t, snapshot, side in records:
        # The labelled row is the searching side's own row 2e+side; the partner is the env's other row, the
        # opponent's own view (train_selfplay.partner_view).
        own, other = 2 * e + side, 2 * e + 1 - side
        np.testing.assert_array_equal(row["ids"], snapshot["ids"][own])
        np.testing.assert_array_equal(row["floats"], snapshot["floats"][own])
        ids, floats = partner_view(snapshot, np.array([other]), torch.device("cpu"))
        np.testing.assert_array_equal(row["partner_ids"], ids[0].numpy())
        np.testing.assert_array_equal(row["partner_floats"], floats[0].numpy())
        np.testing.assert_array_equal(row["foe_match"], snapshot["foe_match"][own])
        assert row["value"] == pytest.approx(float(row["x"] @ row["q"]), abs=1e-5)
        ends = np.flatnonzero(dones[t:, e])
        if len(ends):
            end = t + int(ends[0])
            assert id(row) not in pending_at_end
            assert row["outcome"] == rewards[end, own] and row["remaining_steps"] == end - t
            assert row["outcome"] * .995 ** row["remaining_steps"] == pytest.approx(returns[t, own], abs=1e-6)
            finished += 1
        else:  # still running at close: capped
            assert id(row) in pending_at_end
            assert np.isnan(row["outcome"]) and row["remaining_steps"] == -1
            capped += 1
    assert finished > 20 and capped > 0
    outcomes = {float(r["outcome"]) for r in released if np.isfinite(r["outcome"])}
    assert outcomes <= {-1.0, 0.0, 1.0} and len(outcomes) >= 2
    arrays = {k: np.stack([r[k] for r in released]) for k in released[0]}
    DistillStream._validate_rows(arrays)


def test_an_aborted_battle_releases_capped_rows():
    opponent = searcher()
    opponent.steps = np.array([3, 5, 2])
    opponent.pending = {0: [({}, 1.0, "t", 1, 1)], 1: [({}, 1.0, "t", 2, 1), ({}, 1.0, "t", 5, 0)],
                        2: [({}, 1.0, "t", 0, 1)]}
    opponent.end_step(np.array([True, True, False]), np.array([-1, 0, -1]), np.array([True, False, False]))
    rows = [row for row, _, _ in opponent.labels.rows]
    assert np.isnan(rows[0]["outcome"]) and rows[0]["remaining_steps"] == -1
    assert [(float(r["outcome"]), int(r["remaining_steps"])) for r in rows[1:]] == [(-1.0, 3), (1.0, 0)]
    assert opponent.steps.tolist() == [0, 0, 3] and list(opponent.pending) == [2]


def test_a_different_size_value_model_gives_the_leaf_values(monkeypatch):
    torch.manual_seed(1)
    dex = load_dex()
    policy, critic = Model(dex, 32, 1).eval(), Model(dex, 48, 2).eval()
    calls = []
    original = critic.value

    def spy(state, ids, floats):
        calls.append(len(ids))
        return original(state, ids, floats)

    def forbidden(*args):
        raise AssertionError("the policy's value head must not give leaf values")

    monkeypatch.setattr(critic, "value", spy)
    monkeypatch.setattr(policy, "value", forbidden)
    records, *_ = play(searcher(critic), policy, 12)
    assert records and sum(calls) > 10 * len(calls) > 0
    monkeypatch.undo()
    # The leaf values change the search: the same positions give other pair values than the policy's value head.
    first = play(searcher(critic), policy, 1)[0]
    default = play(searcher(), policy, 1)[0]
    assert len(first) == len(default)
    assert any(not np.array_equal(a[0]["q"], b[0]["q"]) for a, b in zip(first, default))


def test_default_path_is_unchanged_by_the_value_model_option():
    """No value model = the policy's own value head, bit for bit (train_selfplay --search-fraction)."""
    torch.manual_seed(2)
    model = Model(load_dex(), 32, 1).eval()
    default = play(searcher(), model, 40)
    explicit = play(searcher(model), model, 40)
    assert default[3] == explicit[3]  # every choice
    assert len(default[0]) == len(explicit[0]) > 20
    for (a, *_), (b, *_) in zip(default[0], explicit[0]):
        for key in ("x", "q", "prior", "value", "pairs"):
            np.testing.assert_array_equal(a[key], b[key])


def test_shallow_labels_shards_train_and_score_in_distill_train(tmp_path):
    dex = load_dex()
    torch.manual_seed(3)
    teacher, value = tmp_path / "teacher.pt", tmp_path / "value.pt"
    torch.save(dict(model=Model(dex, 32, 1).state_dict(), width=32, layers=1, minutes=7.0), teacher)
    torch.save(dict(model=Model(dex, 48, 2).state_dict(), width=48, layers=2), value)
    out = tmp_path / "labels"
    totals = shallow_labels.main(["--checkpoint", str(teacher), "--value-checkpoint", str(value), "--workers", "1",
                                  "--envs", "4", "--steps", "16", "--labels", "40", "--minutes", "3", "--shard", "16",
                                  "--out", str(out), "--nice", "0", "--status-seconds", "0",
                                  "--search", json.dumps(dict(iterations=50))])
    shards = sorted(out.glob("labels-*.npz"))
    assert totals["labels"] >= 40 and totals["written"] == totals["labels"] + totals["capped_at_close"]
    assert sum(DistillStream._inspect(p)["size"] for p in shards) == totals["written"]
    finished = []
    for path in shards:
        raw = DistillStream._read(path)
        assert str(raw["teacher"]) == "teacher.pt" and float(raw["teacher_minutes"]) == 7.0
        finished.append(np.isfinite(raw["outcome"]).sum())
    assert (out / "status.jsonl").exists() and json.loads((out / "config.json").read_text())["workers"] == 1
    assert DistillData([out], torch.device("cpu")).size > 0
    held = shards[int(np.argmax(finished))]
    args = trainer.parse_args(["train", "--labels", str(out), "--heldout", str(held), "--output",
                               str(tmp_path / "student"), "--device", "cpu", "--width", "32", "--layers", "1",
                               "--epochs", "1", "--batch", "16"])
    result = trainer.train(args)
    assert result["heldout"]["value_rows"] > 0 and np.isfinite(result["heldout"]["value_mse"])
    metrics = trainer.score(Model(dex, 32, 1), [(held, DistillStream._inspect(held))], torch.device("cpu"))
    assert metrics["value_rows"] == finished[int(np.argmax(finished))]


@pytest.mark.parametrize("explore", [0.0, 0.5])
def test_actors_search_both_sides_and_explore_with_the_learner_sample(tmp_path, explore):
    """search_both: both rows of a SEARCH env play the search (forced one-hot: log-prob 0) unless explored, when
    they play the learner's own temperature-1 sample; every searched decision of both sides is labelled."""
    from dataclasses import asdict

    from training.actors import Actors
    from training.league import SEARCH

    torch.manual_seed(4)
    model = Model(load_dex(), 32, 1)
    with Actors(4, 24, [POOL], 3, 1, model, search=asdict(OpponentSearch(iterations=50)), search_labels=tmp_path,
                search_both=True, search_explore=explore) as actors:
        actors.set_weights("learner", model.state_dict())
        actors.set_weights("opponent", model.state_dict())
        actors.start(0, np.full(4, SEARCH), 0.0, minutes=1.0, label_teacher="t.pt")
        collected = actors.wait()
        arrays = actors.buffers[0].arrays
        live = arrays["mask0"][..., :46].any(-1)  # rows that had to act
        sampled = live & (arrays["logp"] < -1e-6)  # a forced search pick has log-prob exactly 0
        for side in (0, 1):
            assert live[:, side::2].sum() > 20
            if explore:
                assert sampled[:, side::2].sum() > 0.2 * live[:, side::2].sum()
            else:
                assert not sampled[:, side::2].any()
    assert (collected["search_explored"] > 0) == (explore > 0)
    written = sum(len(np.load(p)["x"]) for p in tmp_path.glob("labels-*.npz"))
    assert written == collected["search_decisions"] > live.sum() * 0.6
