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
from training.search_opponent import LabelWriter, OpponentSearch, SearchOpponent, load_bench_sets

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


def test_writer_bounds_all_teacher_buffers_and_keeps_rows_after_failed_rename(tmp_path, monkeypatch):
    writer = LabelWriter(tmp_path, size=8)
    for i in range(40):
        writer.add({"outcome": np.float32(i)}, float(i), f"teacher-{i}")
        assert writer.buffered < writer.size
    writer.flush()
    assert writer.written == 40 and not writer.groups
    paths = list(tmp_path.glob("labels-*.npz"))
    assert len(paths) == 40
    assert sum(bool(np.load(p)["holdout"]) for p in paths) == 2
    assert {str(np.load(p)["teacher"]) for p in paths} == {f"teacher-{i}" for i in range(40)}
    writer.add({"outcome": np.float32(41)}, 41.0, "last")
    import training.search_opponent as search_module

    def fail(*args):
        raise OSError("rename failed")

    with monkeypatch.context() as patch:
        patch.setattr(search_module.os, "replace", fail)
        with pytest.raises(OSError, match="rename failed"):
            writer.flush()
    assert writer.buffered == 1 and writer.written == 40
    assert len(list(tmp_path.glob("labels-*.npz"))) == 40
    writer.flush()
    assert writer.written == 41 and not writer.buffered
    assert not list(tmp_path.glob("*.tmp"))


def battle_search(battle, *, fair=True, side=0, bench=BENCH):
    """Search a directed real battle using only player-view observations for the encoder."""
    from types import SimpleNamespace

    encoder = Encoder(2)
    b = encoder.encode_batch([dict(battle_id=1, request=battle.request_json(s), log=battle.drain_player_log(s))
                              for s in (0, 1)])
    torch.manual_seed(5)
    model = Model(load_dex(), 32, 1).eval()
    opponent = SearchOpponent(str(load_dex().path), MoveGuards(), OpponentSearch(iterations=20), seed=3,
                              labels=Capture(), bench_sets=bench, fair_labels=fair)
    with torch.no_grad():
        state = model.trunk(*(torch.from_numpy(getattr(b, k)) for k in OBS_KEYS))
    picks = opponent.act(model, SimpleNamespace(clone_battle=lambda e: battle.clone()), encoder, state,
                         {**b.arrays(), "guard": b.guard}, np.array([0]),
                         np.array([[battle.needs_action(0), battle.needs_action(1)]]), 1.0, side=side)
    return opponent, picks, b, model, state


@pytest.mark.parametrize("case", ["trapping", "imprison", "battle-start", "failed-swap"])
def test_fair_labels_exclude_hidden_masks_and_unreplaced_bench(case):
    from test_protocol import FILLER, PARTNER, PIKACHU, directed_battle, mon

    raichu = mon("Raichu", ["protect", "thunderbolt", "nastyplot", "encore"], "Lightning Rod", "F")
    foe = mon("Gothitelle", ["fakeout", "protect", "psychic", "trickroom"], "Shadow Tag", "F")
    bench = BENCH
    if case == "imprison":
        foe = mon("Mew", ["imprison", "protect", "psychic", "hypervoice"], "Synchronize", "N")
        raichu = mon("Farigiraf", ["protect", "psychic", "hypervoice"], "Armor Tail")
    elif case in ("battle-start", "failed-swap"):
        foe = PIKACHU
    hidden = list(FILLER)
    if case == "battle-start":
        hidden[0] = mon("Zacian", ["ironhead", "protect", "swordsdance", "playrough"], "Intrepid Sword").replace(
            "Leftovers", "Rusted Sword")
    elif case == "failed-swap":
        bench = ([PIKACHU], ["pikachu"])  # the only donor duplicates a seen species
    battle = directed_battle([PIKACHU, raichu] + FILLER, [foe, PARTNER] + hidden)
    if case == "imprison":
        battle.choose(0, "move 2, move 1")
        battle.choose(1, "move 1, move 1")
    opponent, picks, *_ = battle_search(battle, bench=bench)
    assert 0 in picks and opponent.stats["searched"] == 1
    assert opponent.stats["rejected_labels"] == 1 and not opponent.pending


def test_fair_label_inputs_masks_and_priors_match_public_observation():
    from search_labels import make_label
    from training.protocol import request_mask
    from training.search import pair_candidates
    from test_protocol import FILLER, PARTNER, PIKACHU, directed_battle, mon

    raichu = mon("Raichu", ["protect", "thunderbolt", "nastyplot", "encore"], "Lightning Rod", "F")
    battle = directed_battle([PIKACHU, raichu] + FILLER, [PIKACHU, PARTNER] + FILLER)
    opponent, _, b, model, state = battle_search(battle)
    row = opponent.pending[0][0][0]
    masks = request_mask(battle.request_json(0))
    from training.search import _take
    candidates = pair_candidates(model, _take(state, torch.tensor([0])), b.guard[:1], [masks], 4, 8, .5,
                                 MoveGuards())[0]
    public = make_label(dict(side=0, arrays=b.arrays() | {"guard": b.guard}, mask0=masks[0], joint=masks[1],
                             pairs=candidates[0], full=candidates[2], x=row["x"], q=row["q"]), 8)
    for key, expected in public.items():
        np.testing.assert_array_equal(row[key], expected)


def test_actors_accept_teacher_feature_widths_and_close_during_collection(tmp_path):
    from dataclasses import asdict
    from training.actors import Actors
    from training.league import SEARCH

    torch.manual_seed(6)
    model = Model(load_dex(), 32, 1, action_features=0, token_features=0)
    actors = Actors(2, 8, [POOL], 3, 1, model, search=asdict(OpponentSearch(iterations=20)), search_labels=tmp_path,
                    search_both=True)
    actors.set_weights("learner", model.state_dict())
    actors.set_weights("opponent", model.state_dict())
    actors.start(0, np.full(2, SEARCH), 0.0)
    closed = actors.close()  # receive both the rollout stats and the close acknowledgement
    assert all(not p.is_alive() for p in actors.processes)
    shards = list(tmp_path.glob("labels-*.npz"))
    assert shards and sum(DistillStream._inspect(p)["size"] for p in shards) > 10
    assert closed["search_written"] == sum(DistillStream._inspect(p)["size"] for p in shards)
    assert not actors.pending


@pytest.mark.parametrize("stop", ["STOP", "SIGTERM"])
def test_follow_retries_races_checks_feature_shapes_and_drains_stop(tmp_path, monkeypatch, stop):
    import signal

    old = shallow_labels.Teacher(tmp_path / "m001.pt", 1.0, (1, 1, 0))
    new = shallow_labels.Teacher(tmp_path / "m002.pt", 2.0, (1, 2, 0))
    wrong = shallow_labels.Teacher(tmp_path / "m003.pt", 3.0, (1, 3, 0))
    model = Model(load_dex(), 32, 1)
    narrow = Model(load_dex(), 32, 1, token_features=0)
    calls, starts, weights, closed = [], [], [], []
    candidates = iter([old, old, new, new, wrong, wrong])
    failed = False

    def follow(directory, current=None):
        calls.append(current)
        return next(candidates)

    def load(path):
        nonlocal failed
        if path == new.path and not failed:
            failed = True
            raise FileNotFoundError("pruned between metadata and model load")
        selected = narrow if path == wrong.path else model
        return selected, dict(model=selected.state_dict())

    crash = dict(message="injected BatchEnv abort", seed=[1, 2, 3, 4])

    class FakeActors:
        processes = []

        def __init__(self, *args, **kwargs):
            assert kwargs["search_value"] == tmp_path / "value.pt"
            assert kwargs["search_fair_labels"]

        def set_weights(self, key, state):
            weights.append(key)

        def start(self, *args, **kwargs):
            starts.append(kwargs["label_teacher"])

        def wait(self):
            if len(starts) == 5:
                if stop == "STOP":
                    (tmp_path / "out" / "STOP").touch()
                else:
                    signal.getsignal(signal.SIGTERM)(signal.SIGTERM, None)
            return dict(search_decisions=4, search_forced=0, search_explored=1, search_labels=0, completed=0,
                        crashes=[crash] if len(starts) == 1 else [], search_pending=3, search_written=0)

        def close(self):
            closed.append(True)
            # A trainer may already have retired the shards; accounting comes from writer acknowledgements.
            return dict(search_capped_at_close=3, search_written=3)

    monkeypatch.setattr(shallow_labels, "follow_teacher", follow)
    monkeypatch.setattr(shallow_labels, "load_model", load)
    monkeypatch.setattr(shallow_labels, "Actors", FakeActors)
    previous = signal.getsignal(signal.SIGTERM)
    totals = shallow_labels.main(["--follow", str(tmp_path), "--out", str(tmp_path / "out"), "--nice", "0",
                                  "--value-checkpoint", str(tmp_path / "value.pt")])
    assert signal.getsignal(signal.SIGTERM) is previous
    assert calls == [None, old, old, old, new, wrong]
    assert starts == [old.path.name, old.path.name, new.path.name, new.path.name, new.path.name]
    assert weights == ["learner", "opponent", "learner"] and closed == [True]
    assert totals["written"] == totals["capped_at_close"] == 3 and totals["pending"] == 0
    assert totals["crashes"] == 1
    assert json.loads((tmp_path / "out" / "crashes.jsonl").read_text()) == crash


@pytest.mark.parametrize("label_only", [False, True])
def test_actor_exception_flushes_pending_labels(tmp_path, label_only, monkeypatch):
    from dataclasses import asdict
    from training.actors import SharedArrays, _actor, buffer_layout, weight_layout, write_weights
    from training.league import SEARCH

    model = Model(load_dex(), 32, 1)
    layout = weight_layout(model)
    size = sum(int(np.prod(shape)) for _, shape, _ in layout)
    buffers = [SharedArrays(buffer_layout(8, 4)) for _ in range(2)]
    weights = SharedArrays({key: ((size,), np.float32) for key in ("learner", "opponent")})
    for key in weights.arrays:
        write_weights(weights.arrays[key], layout, model.state_dict())
    replies = []

    class Connection:
        def recv(self):
            if replies:
                raise RuntimeError("injected actor failure")
            return "collect", dict(buffer=0, learner_version=0, opponent_version=0, opponents=[SEARCH] * 2,
                                   ko_bonus=0.0, minutes=1.0)

        def send(self, reply):
            replies.append(reply)

    config = dict(size=size, width=32, layers=1, joint_features=False, action_features=model.n_action_features,
                  token_features=model.n_token_features, guards=asdict(MoveGuards()), search=asdict(OpponentSearch()),
                  search_labels=str(tmp_path), search_both=True, label_only=label_only)
    value_calls = []
    if label_only:
        critic = Model(load_dex(), 48, 2)
        value_path = tmp_path / "value.pt"
        torch.save(dict(model=critic.state_dict(), width=48, layers=2), value_path)
        config["search_value"] = str(value_path)
        original = Model.value

        def critic_only(self, *args):
            assert self.width == 48, "label-only mode must never evaluate unused policy values"
            value_calls.append(True)
            return original(self, *args)

        monkeypatch.setattr(Model, "value", critic_only)
    try:
        with pytest.raises(RuntimeError, match="injected actor failure"):
            _actor(Connection(), [b.names() for b in buffers], weights.names(), 2, 8, 0, 2, [POOL], 11,
                   str(Path(__file__).resolve().parents[1]), config)
    finally:
        for buffer in buffers:
            buffer.close(unlink=True)
        weights.close(unlink=True)
    shards = list(tmp_path.glob("labels-*.npz"))
    assert sum(DistillStream._inspect(p)["size"] for p in shards) == replies[0]["search_decisions"] > 10
    assert all(np.isnan(DistillStream._read(p)["outcome"]).all() for p in shards)
    if label_only:
        assert value_calls


def test_exploration_plays_policy_samples_but_keeps_the_search_labels(tmp_path):
    from dataclasses import asdict
    from training.actors import Actors
    from training.league import SEARCH

    torch.manual_seed(7)
    model = Model(load_dex(), 32, 1).eval()
    expected = play(searcher(seed=3007), model, 1, count=4, seed=3000, sides=(0, 1))[0]
    with Actors(4, 1, [POOL], 3, 1, model, search=asdict(OpponentSearch(iterations=50)), search_labels=tmp_path,
                search_both=True, search_explore=1.0) as actors:
        actors.set_weights("learner", model.state_dict())
        actors.set_weights("opponent", model.state_dict())
        actors.start(0, np.full(4, SEARCH), 0.0)
        collected = actors.wait()
        arrays = {k: v.copy() for k, v in actors.buffers[0].arrays.items()}
    assert collected["search_explored"] == 8
    raw = [DistillStream._read(p) for p in tmp_path.glob("labels-*.npz")]
    labels = {k: np.concatenate([s[k] for s in raw]) for k in raw[0] if k in expected[0][0]}
    changed = 0
    for row, e, _, _, side in expected:
        match = np.flatnonzero((labels["ids"] == row["ids"]).all((1, 2)) &
                               (labels["floats"] == row["floats"]).all((1, 2)))
        assert len(match) == 1
        for key in ("pairs", "x", "q", "prior"):
            np.testing.assert_array_equal(labels[key][match[0]], row[key])
        chosen = np.array([arrays["a0"][0, 2 * e + side], arrays["a1"][0, 2 * e + side]])
        changed += not np.array_equal(chosen, row["pairs"][row["x"].argmax()])
    assert changed > 0
    with torch.no_grad():
        state = model.trunk(*(torch.from_numpy(arrays[k][0]) for k in OBS_KEYS))
        lp0, vectors = model.slot0(state, torch.from_numpy(arrays["mask0"][0]))
        lp1 = model.slot1(state, vectors, torch.from_numpy(arrays["a0"][0]), torch.from_numpy(arrays["mask1"][0]))
    np.testing.assert_allclose(arrays["logp"][0], lp0.numpy()[np.arange(8), arrays["a0"][0]] +
                               lp1.numpy()[np.arange(8), arrays["a1"][0]], atol=1e-6)


def test_label_only_actors_keep_identical_labels_without_rollout_buffers(tmp_path):
    from dataclasses import asdict
    from training.actors import Actors
    from training.league import SEARCH

    torch.manual_seed(8)
    model = Model(load_dex(), 32, 1).eval()
    critic = Model(load_dex(), 48, 2).eval()
    value_path = tmp_path / "value.pt"
    torch.save(dict(model=critic.state_dict(), width=48, layers=2), value_path)
    results = []
    for label_only in (False, True):
        out = tmp_path / str(label_only)
        with Actors(2, 12, [POOL], 3, 1, model, search=asdict(OpponentSearch(iterations=20)), search_labels=out,
                    search_value=value_path, search_both=True, search_explore=.5, search_fair_labels=True,
                    label_only=label_only) as actors:
            assert all(bool(b.arrays) != label_only for b in actors.buffers)
            actors.set_weights("learner", model.state_dict())
            actors.set_weights("opponent", model.state_dict())
            actors.start(0, np.full(2, SEARCH), 0.0)
            stats = actors.wait()
        shards = [DistillStream._read(p) for p in sorted(out.glob("labels-*.npz"))]
        results.append(({k: np.concatenate([s[k] for s in shards]) for k in shards[0] if shards[0][k].ndim}, stats))
    assert results[0][0] and results[0][0].keys() == results[1][0].keys()
    for key, expected in results[0][0].items():
        np.testing.assert_array_equal(results[1][0][key], expected)
    for key in ("search_decisions", "search_forced", "search_explored", "search_pending", "search_rejected_labels"):
        assert results[0][1][key] == results[1][1][key]
