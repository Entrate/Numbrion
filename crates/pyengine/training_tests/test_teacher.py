"""Tests for the smart player as a teacher: slot-b labels for a caller's slot a (``second_given``, training.smart),
the labels in the actors' rollouts (training.actors) and the fading imitation loss (train_selfplay)."""
import sys
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

import numbrion as nb
from training import smart
from training.dex import load_dex
from training.encoder import Buffers, Encoder
from training.league import HEURISTIC, SELF, SNAPSHOT
from training.model import Model, masked_log_softmax
from training.scripted import legal_random

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"
PASS, N_ACTIONS = 46, 47
N_ENVS = 8


def agent_type(backend):
    if backend == "rust" and not hasattr(nb, "SmartAgent"):
        pytest.skip("wheel lacks the Rust smart player")
    return smart.PySmartHeuristicAgent if backend == "python" else smart.RustSmartHeuristicAgent


def boundaries(count=60, seed=31):
    """Decision boundaries of random-play battles: (env, encoded arrays, rows that act, their slot-a masks). The env
    steps once the caller asks for the next boundary."""
    torch.set_num_threads(1)
    env = nb.BatchEnv(N_ENVS, [str(POOL)], seed=seed, threads=1, log=True)
    encoder = Encoder(2 * N_ENVS)
    result = env.reset()
    for _ in range(count):
        arrays = encoder.encode_batch([env.observe(e, s) for e in range(N_ENVS) for s in (0, 1)]).arrays()
        rows = np.flatnonzero(result["needs_action"].reshape(-1))
        yield env, arrays, rows, result["mask0"].reshape(-1, N_ACTIONS)[rows].copy()
        result = env.step(env.random_actions(switch_prob=0.15, tera_prob=0.2))


def slot_b_mask(env, rows, a0):
    """The engine's slot-b masks of ``rows`` after their slot-a codes ``a0``."""
    joint = np.full(2 * N_ENVS, -1, dtype=np.int32)
    joint[rows] = a0
    return env.mask_slot1(joint.reshape(N_ENVS, 2)).reshape(-1, N_ACTIONS)[rows].copy()


# ---- second_given ---------------------------------------------------------------------------------------------------

@pytest.mark.parametrize("backend", ("python", "rust"))
def test_second_given_own_slot_a_equals_second(backend):
    """Twins on the same random stream, with all of smart's randomness on: answering its own slot a, second_given
    returns exactly what second does."""
    agent = agent_type(backend)
    a, b = agent(np.random.default_rng(5), temperature=0.05), agent(np.random.default_rng(5), temperature=0.05)
    compared = 0
    for env, arrays, rows, mask0 in boundaries():
        if not len(rows):
            continue
        own = a.first(arrays, rows, mask0)
        np.testing.assert_array_equal(b.first(arrays, rows, mask0), own)
        mask1 = slot_b_mask(env, rows, own)
        np.testing.assert_array_equal(b.second_given(rows, own, mask1), a.second(rows, mask1))
        compared += len(rows)
    assert compared > 500


def test_second_given_answers_the_callers_slot_a(monkeypatch):
    """For another player's slot-a codes (a random learner's), the labels are legal under that player's slot-b mask,
    Rust matches the Python reference, and first's own choice still drives a later second."""
    if not hasattr(nb, "SmartAgent"):
        pytest.skip("wheel lacks the Rust smart player")
    monkeypatch.setattr(smart, "NOISE", 0.0)
    monkeypatch.setattr(smart, "PROTECT_SKIP", 0.0)
    monkeypatch.setattr(smart, "PROTECT_SHARE", (0.7, 0.7))
    reference = smart.PySmartHeuristicAgent(np.random.default_rng(5))
    native = smart.RustSmartHeuristicAgent(np.random.default_rng(5))
    rng = np.random.default_rng(3)
    agreed, total, other = [0, 0], 0, 0
    for env, arrays, rows, mask0 in boundaries():
        if not len(rows):
            continue
        learner = legal_random(mask0, rng)
        own = native.first(arrays, rows, mask0)
        own_reference = reference.first(arrays, rows, mask0)
        mask1 = slot_b_mask(env, rows, learner)
        labels = native.second_given(rows, learner, mask1)
        assert labels.dtype == np.int32 and mask1[np.arange(len(rows)), labels].all()
        agreed[0] += int((reference.second_given(rows, learner, mask1) == labels).sum())
        np.testing.assert_array_equal(reference.a0, own_reference)  # restored after second_given
        own_mask1 = slot_b_mask(env, rows, own)
        agreed[1] += int((reference.second(rows, own_mask1) == native.second(rows, own_mask1)).sum())
        total += len(rows)
        other += int((learner != own).sum())
    assert total > 500 and other > 100
    assert min(agreed) / total >= 0.995, (agreed, total)


def test_second_given_input_validation():
    if not hasattr(nb, "SmartAgent"):
        pytest.skip("wheel lacks the Rust smart player")
    native = nb.SmartAgent(str(load_dex().path), 0)
    b = Buffers(2)
    arrays = tuple(getattr(b, k) for k in smart.OBS_KEYS)
    rows, mask = np.array([0], dtype=np.int64), np.zeros((1, N_ACTIONS), dtype=bool)
    mask[:, PASS] = True
    with pytest.raises(ValueError, match="first"):
        native.second_given(np.array([PASS]), mask)
    native.first(*arrays, rows, mask)
    for a0 in (np.array([N_ACTIONS]), np.array([-1]), np.array([PASS, PASS]), np.array([PASS], dtype=np.int32)):
        with pytest.raises(ValueError, match="a0"):
            native.second_given(a0, mask)
    with pytest.raises(ValueError, match="mask1"):
        native.second_given(np.array([PASS, PASS]), np.ones((2, N_ACTIONS), dtype=bool))
    assert native.second_given(np.array([PASS]), mask).tolist() == [PASS]
    python = smart.PySmartHeuristicAgent(np.random.default_rng(0))
    with pytest.raises(ValueError, match="slot-a code per row"):
        python.second_given(rows, [PASS], mask)


# ---- the imitation loss and its schedule ----------------------------------------------------------------------------

def test_teacher_coefficient_fades_linearly_to_zero():
    from train_selfplay import teacher_coefficient

    args = SimpleNamespace(teacher_coef=0.3, teacher_end_minutes=30.0)
    assert teacher_coefficient(args, 0.0) == pytest.approx(0.3)
    assert teacher_coefficient(args, 15 * 60) == pytest.approx(0.15)  # elapsed includes a resumed run's earlier time
    assert teacher_coefficient(args, 30 * 60) == 0.0 and teacher_coefficient(args, 90 * 60) == 0.0
    assert teacher_coefficient(SimpleNamespace(teacher_coef=0.0, teacher_end_minutes=30.0), 0.0) == 0.0
    assert teacher_coefficient(SimpleNamespace(teacher_coef=0.3, teacher_end_minutes=0.0), 0.0) == 0.0
    assert teacher_coefficient(SimpleNamespace(), 0.0) == 0.0  # learner_bench's args


def test_ppo_loss_adds_the_mean_label_cross_entropy():
    from train_selfplay import ppo_loss

    torch.manual_seed(0)
    mask0, mask1 = torch.rand(4, N_ACTIONS) < 0.5, torch.rand(4, N_ACTIONS) < 0.5
    teacher_a0, teacher_a1 = torch.tensor([3, 0, 5, 7]), torch.tensor([9, 0, 2, 4])  # row 1: unlabelled
    a0, a1 = torch.tensor([3, 1, 5, 8]), torch.tensor([9, 2, 2, 4])
    for m, codes in ((mask0, teacher_a0), (mask0, a0), (mask1, teacher_a1), (mask1, a1)):
        m[torch.arange(4), codes] = True
    mask0[1, 0] = mask1[1, 0] = False  # the unlabelled row's placeholder code is illegal: its -1e9 must not leak
    logits0 = torch.randn(4, N_ACTIONS, dtype=torch.float64, requires_grad=True)
    logits1 = torch.randn(4, N_ACTIONS, dtype=torch.float64, requires_grad=True)
    zero = torch.zeros(4, dtype=torch.float64)

    class Policy:
        def trunk(self, *args):
            return None

        def slot0(self, state, mask):
            return masked_log_softmax(logits0, mask), None

        def slot1(self, state, vectors, a0, mask):
            return masked_log_softmax(logits1, mask)

        def value(self, *args):
            return zero

        def aux_loss(self, *args):
            return zero.sum()

    batch = dict.fromkeys(("ids", "floats", "field", "action_features", "token_features", "foe_match"))
    common = (Policy(), batch, None, None, a0, a1, mask0, mask1, zero - 1.0, torch.ones(4, dtype=torch.float64), zero,
              0.5, 0.01, 0.03, 0.2)
    plain, plain_terms = ppo_loss(*common)
    labelled = torch.tensor([1.0, 0.0, 1.0, 1.0], dtype=torch.float64)
    coef = torch.tensor(0.3, dtype=torch.float64)
    loss, terms = ppo_loss(*common, teacher_a0=teacher_a0, teacher_a1=teacher_a1, teacher_labelled=labelled,
                           teacher_coef=coef)
    lp0, lp1 = masked_log_softmax(logits0, mask0), masked_log_softmax(logits1, mask1)
    rows = [0, 2, 3]
    expected = -(lp0[rows, teacher_a0[rows]] + lp1[rows, teacher_a1[rows]]).mean()
    assert len(terms) == len(plain_terms) + 1
    torch.testing.assert_close(terms[-1], expected.detach())
    torch.testing.assert_close(loss, plain + 0.3 * expected)
    assert all(torch.equal(a, b) for a, b in zip(terms, plain_terms))
    (loss - plain).backward()
    assert not logits0.grad[1].any() and not logits1.grad[1].any()  # the unlabelled row gets no imitation gradient
    assert logits0.grad[0, teacher_a0[0]] < 0 and logits1.grad[0, teacher_a1[0]] < 0  # labels are pushed up
    # A minibatch without labels: a zero term and the plain loss, exactly.
    loss, terms = ppo_loss(*common, teacher_a0=teacher_a0, teacher_a1=teacher_a1,
                           teacher_labelled=torch.zeros_like(labelled), teacher_coef=coef)
    assert float(terms[-1]) == 0.0 and torch.equal(loss, plain)


# ---- actors and the update ------------------------------------------------------------------------------------------

@pytest.fixture(scope="module")
def rollouts():
    """The same two rollouts without teacher buffers and with them (labelling on, then off), from one model."""
    from train_selfplay import cpu_state
    from training.actors import Actors

    torch.set_num_threads(1)
    torch.manual_seed(0)
    model = Model(load_dex(), width=32, layers=1)
    state = cpu_state(model)
    opponents = np.array([SELF, SNAPSHOT, HEURISTIC, HEURISTIC], dtype=np.int8)
    out = {}
    for teacher in (False, True):
        with Actors(4, 24, [POOL], seed=3, workers=1, model=model, scripted="smart", scripted_temperature=0.05,
                    teacher=teacher) as actors:
            actors.set_weights("learner", state)
            actors.set_weights("opponent", state)
            out[teacher] = []
            for buffer, teach in enumerate((teacher, False)):
                actors.start(buffer, opponents, ko_bonus=0.02, teacher=teach)
                totals = actors.wait()
                assert (totals["teacher_seconds"] > 0) == teach
                out[teacher].append({k: v.copy() for k, v in actors.buffers[buffer].arrays.items()})
    return out[False], out[True], state


def test_labelling_changes_nothing_the_learner_plays(rollouts):
    plain, taught, _ = rollouts
    for before, after in zip(plain, taught):
        assert set(after) - set(before) == {"teacher_a0", "teacher_a1"}
        for key, value in before.items():
            np.testing.assert_array_equal(after[key], value, err_msg=key)


def test_actors_label_every_trained_decision_legally(rollouts):
    _, (labelled_rollout, unlabelled_rollout), _ = rollouts
    data = labelled_rollout
    labelled = data["teacher_a0"] >= 0
    assert labelled.any() and (labelled == data["train"]).all() and ((data["teacher_a1"] >= 0) == labelled).all()
    assert (data["teacher_a0"][~labelled] == -1).all() and (data["teacher_a1"][~labelled] == -1).all()
    steps, rows = np.nonzero(labelled)
    assert data["mask0"][steps, rows, data["teacher_a0"][labelled]].all()  # the learner's guarded masks
    assert data["mask1"][steps, rows, data["teacher_a1"][labelled]].all()  # after the learner's own slot a
    assert (unlabelled_rollout["teacher_a0"] == -1).all() and (unlabelled_rollout["teacher_a1"] == -1).all()


def test_ppo_update_imitates_the_labels(rollouts):
    from train_selfplay import ppo_update
    from training.ppo import Adam

    _, (data, unlabelled), state = rollouts
    args = SimpleNamespace(gamma=0.995, lam=0.95, clip=0.2, value_coef=0.5, aux_coef=0.03, tera_entropy_weight=0.0,
                           max_grad_norm=0.5, epochs=2, minibatch=64, teacher_coef=1.0)
    device = torch.device("cpu")

    def updates(data, args, coef, n):
        torch.manual_seed(0)
        model = Model(load_dex(), width=32, layers=1)
        model.load_state_dict(state)
        optimizer, rng = Adam(model.parameters(), lr=3e-3), np.random.default_rng(0)
        return [ppo_update(model, optimizer, data, args, rng, 0.01, device, coef)[0] for _ in range(n)], model

    history, _ = updates(data, args, 1.0, 5)
    losses = [h["teacher_loss"] for h in history]
    assert np.isfinite(losses).all() and losses[-1] < losses[0] - 0.3, losses
    assert history[0]["teacher_coef"] == 1.0
    assert 0 <= history[0]["teacher_agree0"] <= 1 and 0 <= history[0]["teacher_agree1"] <= 1
    # After the fade: weight 0 and no labels leave the plain PPO update, exactly.
    (faded,), faded_model = updates(unlabelled, args, 0.0, 1)
    assert faded["teacher_loss"] is None and faded["teacher_agree0"] is None
    plain_data = {k: v for k, v in unlabelled.items() if not k.startswith("teacher_")}
    (plain,), plain_model = updates(plain_data, SimpleNamespace(**dict(vars(args), teacher_coef=0.0)), 0.0, 1)
    assert "teacher_loss" not in plain
    for (name, a), b in zip(faded_model.state_dict().items(), plain_model.state_dict().values()):
        assert torch.equal(a, b), name


def test_critic_warmup_trains_only_the_critic_and_hidden_information_heads(rollouts):
    from train_selfplay import CRITIC_PREFIXES, ppo_update
    from training.ppo import Adam

    plain, _, state = rollouts
    data = plain[0]
    args = SimpleNamespace(gamma=0.995, lam=0.95, clip=0.2, value_coef=0.5, aux_coef=0.03, tera_entropy_weight=0.0,
                           max_grad_norm=0.5, epochs=2, minibatch=64, critic_warmup_updates=1)
    torch.manual_seed(0)
    model = Model(load_dex(), width=32, layers=1)
    model.load_state_dict(state)
    optimizer, rng = Adam(model.parameters(), lr=3e-3), np.random.default_rng(0)
    ppo_update(model, optimizer, data, args, rng, 0.01, torch.device("cpu"), update=1)
    changed = {name for name, p in model.state_dict().items() if not torch.equal(p, state[name])}
    assert changed and all(name.startswith(CRITIC_PREFIXES) for name in changed), changed
    ppo_update(model, optimizer, data, args, rng, 0.01, torch.device("cpu"), update=2)  # warm-up over: all learn
    assert not torch.equal(model.state_dict()["blocks.0.qkv.weight"], state["blocks.0.qkv.weight"])
