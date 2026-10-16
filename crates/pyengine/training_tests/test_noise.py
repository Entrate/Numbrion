"""Tests for the gradient-noise-scale estimator and the automatic learning-rate rule (training.noise), their place in
the PPO update (train_selfplay.ppo_update), flag-off parity and resume."""
import importlib.util
import json
import math
import subprocess
import sys
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

from training.dex import load_dex
from training.league import HEURISTIC, SELF, SNAPSHOT
from training.model import Model
from training.noise import GradientNoise, LrRule, NoiseScale, noise_estimates

HERE = Path(__file__).resolve().parents[1]
REPO = HERE.parents[1]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"
PRE_FEATURE_COMMIT = "643c385"  # the trainer before the noise-scale flags (numbrion/main, 2026-10-15)


# ---- estimator ------------------------------------------------------------------------------------------------------

def test_noise_estimates_invert_the_expected_moments():
    """E|g_b|^2 = |G|^2 + tr/b and E|G_B|^2 = |G|^2 + tr/B: the estimates recover |G|^2 and tr exactly."""
    g2, trace, b, big = 0.25, 300.0, 64, 1024
    assert noise_estimates(g2 + trace / b, g2 + trace / big, b, big) == pytest.approx((g2, trace))
    with pytest.raises(ValueError):
        noise_estimates(1.0, 1.0, 64, 64)


def test_estimator_recovers_the_noise_scale_of_linear_regression():
    """Least squares with x ~ N(0, I_d) and y = x.w* + eps at w = w* + delta: the per-row gradient
    x (x.delta - eps) has mean delta and covariance (|delta|^2 + s^2) I + delta delta^T, so
    B_noise = (d (|delta|^2 + s^2) + |delta|^2) / |delta|^2. Each "update" draws a fresh rollout and partitions it
    into minibatches without replacement, as the trainer's first epoch does."""
    torch.manual_seed(0)
    d, s2, small, m = 40, 3.0, 32, 16
    delta = torch.randn(d)
    delta *= 1 / delta.norm()
    expected = (d * (1 + s2) + 1) / 1
    w = torch.zeros(d, requires_grad=True)
    smoothed = NoiseScale(updates=400)
    raw = []
    for _ in range(400):
        x = torch.randn(m * small, d)
        y = x @ (-delta) + math.sqrt(s2) * torch.randn(m * small)  # w* = -delta, so w - w* = delta at w = 0
        probe = GradientNoise()
        for k in range(m):
            rows = slice(k * small, (k + 1) * small)
            loss = 0.5 * (x[rows] @ w - y[rows]).square().mean()
            probe.add(torch.autograd.grad(loss, [w]), small)
        estimate = probe.estimate()
        assert estimate["small"] == small and estimate["big"] == small * m and estimate["gradients"] == m
        smoothed.observe(estimate["g2"], estimate["trace"])
        raw.append(estimate)
    assert smoothed.b_noise == pytest.approx(expected, rel=0.1)
    assert np.mean([e["trace"] for e in raw]) == pytest.approx(d * (1 + s2) + 1, rel=0.05)
    assert np.mean([e["g2"] for e in raw]) == pytest.approx(1.0, rel=0.1)  # independent rows: unbiased in one update
    summary = smoothed.summary()
    assert summary["noise_measured"] == 400 and summary["noise_b"] == round(smoothed.b_noise)


def test_correlated_rows_bias_random_chunks_but_not_env_chunks():
    """Rows in envs of ``c`` sharing a noise term (like a battle's decisions): row gradient G + u_env + e_row. Random
    chunks of one rollout see the row-level spread, d (s_u^2 + s_e^2), but the rollout mean also carries the envs'
    shared noise: the one-update |G|^2 estimate is too large by about d s_u^2 (c - 1) / B. Chunks of whole envs
    (env_chunks) give an unbiased |G|^2 and the effective trace d (c s_u^2 + s_e^2)."""
    from training.noise import env_chunks

    torch.manual_seed(1)
    d, steps, envs, small = 50, 16, 64, 128  # 2 rows per env and step: c = 32 rows per env
    c, n = 2 * steps, 2 * steps * envs
    s_u2, s_e2 = 1.0, 1.0
    signal = torch.randn(d)
    signal *= 0.5 / signal.norm()  # |G|^2 = 0.25
    selected = np.arange(n)  # flat step * (2 envs) + row indices, all trained
    env_of_row = torch.from_numpy((selected % (2 * envs)) // 2)
    estimates = {"random": [], "env": []}
    for _ in range(300):
        shared = math.sqrt(s_u2) * torch.randn(envs, d)
        rows = signal + shared[env_of_row] + math.sqrt(s_e2) * torch.randn(n, d)
        for kind, chunks in (("random", np.random.permutation(n)[: n // small * small].reshape(-1, small)),
                             ("env", env_chunks(selected, 2 * envs, small))):
            probe = GradientNoise()
            for chunk in chunks:
                probe.add([rows[torch.from_numpy(np.asarray(chunk))].mean(0)], small)
            estimates[kind].append(probe.estimate())
    bias = d * s_u2 * (c - 1) / n
    assert bias > 3 * 0.25 - 0.01  # 3x the signal
    random_g2 = np.mean([e["g2"] for e in estimates["random"]])
    assert random_g2 == pytest.approx(0.25 + bias, rel=0.1)
    assert np.mean([e["g2"] for e in estimates["env"]]) == pytest.approx(0.25, abs=0.06)
    assert np.mean([e["trace"] for e in estimates["env"]]) == pytest.approx(d * (c * s_u2 + s_e2), rel=0.1)


def test_env_chunks_keep_envs_together_in_equal_chunks():
    from training.noise import env_chunks

    rows_per_step, steps = 8, 5  # 4 envs
    flat = np.arange(rows_per_step * steps)
    selected = flat[np.random.default_rng(0).random(len(flat)) < 0.7]  # some rows untrained
    chunks = env_chunks(selected, rows_per_step, 6)
    assert len(chunks) == len(selected) // 6 and all(len(c) == 6 for c in chunks)
    used = np.concatenate(chunks)
    assert len(set(used.tolist())) == len(used)
    envs = (selected[used] % rows_per_step) // 2
    assert (np.diff(envs) >= 0).all()  # in env order: a chunk holds whole envs except at its cuts


def test_gradient_noise_needs_two_equal_batches_of_the_same_parameters():
    probe = GradientNoise()
    probe.add([torch.ones(3), None], 8)
    assert probe.estimate() is None
    with pytest.raises(ValueError):
        probe.add([torch.ones(3)], 16)
    with pytest.raises(ValueError):
        probe.add([torch.ones(4)], 8)
    probe.add([torch.ones(3), None], 8)
    assert probe.estimate()["trace"] == pytest.approx(0.0)  # identical gradients: no noise


def test_noise_scale_is_a_bias_corrected_ema_and_resumes():
    noise = NoiseScale(updates=4, every=3)
    assert noise.b_noise is None and noise.summary()["noise_b"] is None
    assert [noise.due(u) for u in range(1, 7)] == [False, False, True, False, False, True]
    noise.observe(2.0, 100.0)
    assert noise.b_noise == pytest.approx(50.0) and noise.summary()["noise_g2"] == pytest.approx(2.0)
    noise.observe(-6.0, 100.0)  # smoothed signal now negative: unmeasurable
    assert noise.b_noise == math.inf and noise.summary()["noise_b"] is None
    noise.observe(float("nan"), 1.0)  # ignored
    assert noise.measured == 2
    resumed = NoiseScale(updates=4, every=3)
    resumed.load_state_dict(noise.state_dict())
    assert resumed.state_dict() == noise.state_dict()


# ---- the rule -------------------------------------------------------------------------------------------------------

def measured(b_noise, count=1000):
    return SimpleNamespace(b_noise=b_noise, measured=count)


def test_rule_target_and_clamping():
    rule = LrRule(power=0.5, min_factor=0.1, max_change=10.0, b_ref=1000.0)
    assert rule.target(4000.0) == pytest.approx(0.5)
    assert rule.target(250.0) == 1.0  # never above the schedule
    assert rule.target(1e9) == pytest.approx(0.1)  # floor
    assert rule.target(math.inf) == pytest.approx(0.1)  # no measurable signal: the floor
    assert rule.target(None) == 1.0 and rule.target(-5.0) == 1.0  # nothing measured / negative trace: keep
    linear = LrRule(power=1.0, min_factor=0.01, b_ref=1000.0)
    assert linear.target(8000.0) == pytest.approx(0.125)


def test_rule_moves_at_most_max_change_per_measured_update():
    rule = LrRule(power=1.0, min_factor=0.1, max_change=0.25, b_ref=100.0)
    factors = [rule.update(measured(1000.0)) for _ in range(12)]
    assert factors[:3] == pytest.approx([0.8, 0.64, 0.512])
    assert factors[-1] == pytest.approx(0.1)  # reached the target, held at the floor
    assert rule.update(measured(200.0)) == pytest.approx(0.125)  # up again by at most 25%: target 0.5
    assert rule.summary() == dict(lr_factor=0.125, lr_b_ref=100, accumulate=1)


def test_rule_takes_b_ref_after_the_warmup():
    rule = LrRule(power=0.5, warmup=3, max_change=1.0)
    assert rule.update(measured(500.0, count=2)) == 1.0 and rule.b_ref is None
    assert rule.update(measured(math.inf, count=3)) == 1.0 and rule.b_ref is None  # not finite: wait
    assert rule.update(measured(500.0, count=4)) == 1.0 and rule.b_ref == 500.0
    assert rule.update(measured(2000.0, count=5)) == pytest.approx(0.5)


def test_rule_modes_apply_the_factor_to_lr_or_to_the_batch():
    lr = LrRule(b_ref=100.0, power=1.0, max_change=100.0, min_factor=0.05)
    accumulate = LrRule(b_ref=100.0, power=1.0, max_change=100.0, min_factor=0.05, mode="accumulate")
    for rule in (lr, accumulate):
        rule.update(measured(400.0))
    assert lr.lr_factor == pytest.approx(0.25) and lr.accumulate == 1
    assert accumulate.lr_factor == 1.0 and accumulate.accumulate == 4
    accumulate.update(measured(300.0))
    assert accumulate.accumulate == 3
    with pytest.raises(ValueError):
        LrRule(mode="batch")
    with pytest.raises(ValueError):
        LrRule(min_factor=0.0)


def test_rule_resumes_its_factor_and_b_ref():
    rule = LrRule(power=1.0, max_change=0.5, b_ref=100.0)
    rule.update(measured(1000.0))
    resumed = LrRule(power=1.0, max_change=0.5)
    resumed.load_state_dict(rule.state_dict())
    assert resumed.factor == rule.factor and resumed.b_ref == 100.0
    given = LrRule(power=1.0, max_change=0.5, b_ref=300.0)
    given.load_state_dict(rule.state_dict(), keep_b_ref=True)
    assert given.b_ref == 300.0 and given.factor == rule.factor
    other_mode = LrRule(power=1.0, mode="accumulate")
    other_mode.load_state_dict(rule.state_dict())
    assert other_mode.factor == 1.0  # an lr factor is not a batch factor


# ---- in the PPO update ----------------------------------------------------------------------------------------------

@pytest.fixture(scope="module")
def rollout():
    from train_selfplay import cpu_state
    from training.actors import Actors

    torch.set_num_threads(1)
    torch.manual_seed(0)
    model = Model(load_dex(), width=32, layers=1)
    state = cpu_state(model)
    with Actors(4, 24, [POOL], seed=3, workers=1, model=model, scripted="smart", scripted_temperature=0.05) as actors:
        actors.set_weights("learner", state)
        actors.set_weights("opponent", state)
        actors.start(0, np.array([SELF, SELF, SNAPSHOT, HEURISTIC], dtype=np.int8), ko_bonus=0.02)
        actors.wait()
        data = {k: v.copy() for k, v in actors.buffers[0].arrays.items()}
    return data, state


def run_update(update_fn, data, state, args, **kwargs):
    from training.ppo import Adam

    torch.manual_seed(0)
    model = Model(load_dex(), width=32, layers=1)
    model.load_state_dict(state)
    optimizer = Adam(model.parameters(), lr=3e-3)
    stats, samples = update_fn(model, optimizer, data, args, np.random.default_rng(0), 0.01, torch.device("cpu"),
                               update=1, **kwargs)
    return model, optimizer, stats, samples


def default_args(**overrides):
    import train_selfplay

    args = train_selfplay.parse_args(["--minibatch", "64"])
    return SimpleNamespace(**{**vars(args), **overrides})


def assert_same_weights(a, b):
    for (name, x), y in zip(a.state_dict().items(), b.state_dict().values()):
        assert torch.equal(x, y), name


def test_flags_off_update_is_bit_identical_to_the_trainer_before_the_flags(rollout, tmp_path):
    """The update with the new flags at their defaults against ``ppo_update`` from the commit before them.

    Pinned to that commit: once the flags have landed and ``ppo_update`` changes for other reasons, drop this test
    (the probe-parity test below keeps checking that measuring changes nothing)."""
    import train_selfplay

    source = subprocess.run(["git", "-C", str(REPO), "show", f"{PRE_FEATURE_COMMIT}:crates/pyengine/train_selfplay.py"],
                            capture_output=True, text=True)
    if source.returncode:
        pytest.skip(f"commit {PRE_FEATURE_COMMIT} not available")
    path = tmp_path / "train_selfplay_before.py"
    path.write_text(source.stdout)
    spec = importlib.util.spec_from_file_location("train_selfplay_before", path)
    before = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(before)
    data, state = rollout
    args = default_args()
    assert not args.noise_scale and not args.auto_lr
    old_model, old_optimizer, old_stats, old_samples = run_update(before.ppo_update, data, state, args)
    new_model, new_optimizer, new_stats, new_samples = run_update(train_selfplay.ppo_update, data, state, args)
    assert_same_weights(old_model, new_model)
    assert old_stats == new_stats and old_samples == new_samples
    assert old_optimizer.step_count == new_optimizer.step_count
    for a, b in zip(old_optimizer.v, new_optimizer.v):
        assert torch.equal(a, b)


def test_measuring_the_noise_scale_leaves_the_update_unchanged(rollout):
    import train_selfplay

    data, state = rollout
    args = default_args(minibatch=32)
    plain, _, plain_stats, _ = run_update(train_selfplay.ppo_update, data, state, args)
    probe = GradientNoise()
    probed, _, probed_stats, _ = run_update(train_selfplay.ppo_update, data, state, args, noise=probe)
    assert_same_weights(plain, probed)
    assert plain_stats == probed_stats
    trained = int(data["train"].sum())
    assert probe.count == trained // 32 >= 3 and probe.rows == 32  # env chunks of --minibatch rows
    estimate = probe.estimate()
    assert estimate["trace"] > 0 and np.isfinite(estimate["g2"])
    # The probe holds the policy loss's gradient alone: none for the critic and hidden-information heads.
    policy_params = sum(p.numel() for n, p in plain.named_parameters()
                        if not n.startswith(train_selfplay.CRITIC_PREFIXES))
    assert probe.total.numel() == policy_params


def test_probe_measures_at_the_probe_model_and_leaves_the_update_unchanged(rollout):
    """The probe at a perturbed copy (as at the weight average) measures there; the trained model's update is the
    plain one."""
    import train_selfplay

    data, state = rollout
    args = default_args(minibatch=32)
    plain, _, _, _ = run_update(train_selfplay.ppo_update, data, state, args)
    other = Model(load_dex(), width=32, layers=1)
    other.load_state_dict({k: v + 0.05 * torch.randn_like(v) if v.is_floating_point() else v for k, v in state.items()})
    at_other, at_model = GradientNoise(), GradientNoise()
    probed, _, _, _ = run_update(train_selfplay.ppo_update, data, state, args, noise=at_other, probe_model=other)
    run_update(train_selfplay.ppo_update, data, state, args, noise=at_model)
    assert_same_weights(plain, probed)
    assert at_other.count == at_model.count >= 3
    assert not torch.allclose(at_other.total, at_model.total)
    assert all(p.grad is None for p in other.parameters())  # autograd.grad leaves .grad alone


def test_policy_surrogate_is_ppo_loss_policy_term_without_the_clip(rollout):
    import train_selfplay
    from train_selfplay import N_ACTIONS, OBS_KEYS

    data, state = rollout
    model = Model(load_dex(), width=32, layers=1)
    model.load_state_dict(state)
    rows = np.flatnonzero(data["train"].reshape(-1))[:40]
    flat = lambda k, *shape: torch.from_numpy(np.ascontiguousarray(data[k].reshape(-1, *shape)[rows]))  # noqa: E731
    batch = {k: flat(k, *data[k].shape[2:]) for k in OBS_KEYS}
    a0, a1, mask0, mask1 = flat("a0"), flat("a1"), flat("mask0", N_ACTIONS), flat("mask1", N_ACTIONS)
    torch.manual_seed(3)
    old_logp = flat("logp") + 0.5 * torch.randn(len(rows))  # ratios far from 1: the clip would bite
    weight = torch.randn(len(rows))
    one = torch.ones(())
    _, terms = train_selfplay.ppo_loss(model, batch, batch["ids"], batch["floats"][:, 0:6], a0, a1, mask0, mask1,
                                       old_logp, weight, flat("returns") if "returns" in data else weight, one, one,
                                       one, math.inf)
    surrogate = train_selfplay.policy_surrogate(model, batch, a0, a1, mask0, mask1, old_logp, weight)
    torch.testing.assert_close(surrogate.detach(), terms[0])
    _, clipped = train_selfplay.ppo_loss(model, batch, batch["ids"], batch["floats"][:, 0:6], a0, a1, mask0, mask1,
                                         old_logp, weight, weight, one, one, one, 0.2)
    assert not torch.isclose(clipped[0], terms[0])
    grads = torch.autograd.grad(surrogate, list(model.parameters()), allow_unused=True)
    names = [n for (n, _), g in zip(model.named_parameters(), grads) if g is not None]
    assert names and not any(n.startswith(train_selfplay.CRITIC_PREFIXES) for n in names)


def test_policy_surrogate_without_old_logp_is_the_plain_policy_gradient(rollout):
    """``old_logp`` None (the probe at the weight average): ratio 1, gradient of ``-mean(weight * log p)``."""
    import train_selfplay
    from train_selfplay import N_ACTIONS, OBS_KEYS

    data, state = rollout
    model = Model(load_dex(), width=32, layers=1)
    model.load_state_dict(state)
    rows = np.flatnonzero(data["train"].reshape(-1))[:40]
    flat = lambda k, *shape: torch.from_numpy(np.ascontiguousarray(data[k].reshape(-1, *shape)[rows]))  # noqa: E731
    batch = {k: flat(k, *data[k].shape[2:]) for k in OBS_KEYS}
    a0, a1, mask0, mask1 = flat("a0"), flat("a1"), flat("mask0", N_ACTIONS), flat("mask1", N_ACTIONS)
    torch.manual_seed(4)
    weight = torch.randn(len(rows))
    params = list(model.parameters())
    surrogate = train_selfplay.policy_surrogate(model, batch, a0, a1, mask0, mask1, None, weight)
    torch.testing.assert_close(surrogate.detach(), -weight.mean())
    state_ = model.trunk(batch["ids"], batch["floats"], batch["field"], batch["action_features"], batch["token_features"])
    lp0, vectors0 = model.slot0(state_, mask0)
    lp1 = model.slot1(state_, vectors0, a0, mask1)
    logp = (lp0 * train_selfplay.onehot(a0)).sum(-1) + (lp1 * train_selfplay.onehot(a1)).sum(-1)
    expected = torch.autograd.grad(-(weight * logp).mean(), params, allow_unused=True)
    got = torch.autograd.grad(surrogate, params, allow_unused=True)
    assert any(g is not None for g in got)
    for g, e in zip(got, expected):
        assert (g is None) == (e is None)
        if g is not None:
            torch.testing.assert_close(g, e)


def test_no_probe_while_only_the_critic_learns(rollout):
    import train_selfplay

    data, state = rollout
    probe = GradientNoise()
    run_update(train_selfplay.ppo_update, data, state, default_args(critic_warmup_updates=1), noise=probe)
    assert probe.count == 0


def test_accumulation_takes_fewer_steps_and_one_is_plain(rollout):
    import train_selfplay

    data, state = rollout
    args = default_args()
    plain, plain_optimizer, _, _ = run_update(train_selfplay.ppo_update, data, state, args)
    one, _, _, _ = run_update(train_selfplay.ppo_update, data, state, args, accumulate=1)
    assert_same_weights(plain, one)
    _, accumulated, _, _ = run_update(train_selfplay.ppo_update, data, state, args, accumulate=3)
    assert accumulated.step_count == math.ceil(plain_optimizer.step_count / 3)
    # Summing the whole update into one step equals one step on the mean of the minibatch gradients.
    _, single, _, _ = run_update(train_selfplay.ppo_update, data, state, default_args(epochs=1), accumulate=100)
    assert single.step_count == 1


def test_parse_args_defaults_off_and_auto_lr_implies_the_estimator():
    import train_selfplay

    defaults = train_selfplay.parse_args([])
    assert not defaults.noise_scale and not defaults.auto_lr and defaults.auto_lr_mode == "lr"
    args = train_selfplay.parse_args(["--auto-lr", "--auto-lr-bref", "40000", "--ema-updates", "100"])
    assert args.noise_scale and args.auto_lr_bref == 40000 and args.noise_weights == "average"
    assert train_selfplay.parse_args(["--noise-scale"]).noise_weights == "raw"
    assert train_selfplay.parse_args(["--auto-lr", "--noise-weights", "raw"]).noise_weights == "raw"
    with pytest.raises(SystemExit):
        train_selfplay.parse_args(["--auto-lr"])  # the rule measures at the weight average
    with pytest.raises(SystemExit):
        train_selfplay.parse_args(["--noise-scale", "--noise-weights", "average"])
    for bad in (["--auto-lr-min", "0"], ["--auto-lr-power", "-1"], ["--auto-lr-bref", "-3"]):
        with pytest.raises(SystemExit):
            train_selfplay.parse_args(["--auto-lr", "--ema-updates", "100", *bad])
    for bad in (["--noise-every", "0"], ["--noise-ema", "0.5"]):
        with pytest.raises(SystemExit):
            train_selfplay.parse_args(bad)


# ---- resume ---------------------------------------------------------------------------------------------------------

def test_resume_keeps_the_noise_scale_and_lr_rule_state(tmp_path):
    """A tiny run with the rule, then a resumed one: latest.pt holds both states and the resumed run's first update
    continues them (one more measurement, the same B_ref, a factor within one max change of the saved one)."""
    common = [sys.executable, str(HERE / "train_selfplay.py"), "--width", "32", "--layers", "1", "--envs", "8",
              "--steps", "8", "--workers", "1", "--device", "cpu", "--minibatch", "32", "--auto-lr", "--noise-every", "1",
              "--ema-updates", "5",
              "--noise-ema", "3", "--auto-lr-warmup", "2", "--auto-lr-max-change", "0.5", "--auto-lr-power", "1",
              "--output", str(tmp_path), "--schedule-minutes", "10"]
    subprocess.run(common + ["--minutes", "0.06"], check=True, capture_output=True, cwd=HERE)
    saved = torch.load(tmp_path / "latest.pt", map_location="cpu", weights_only=False)
    assert saved["noise_scale"]["measured"] == saved["update"] >= 3
    assert saved["lr_rule"]["b_ref"] is not None and saved["lr_rule"]["mode"] == "lr"
    lines = (tmp_path / "metrics.jsonl").read_text().splitlines()
    assert len(lines) == saved["update"]
    subprocess.run(common + ["--minutes", "0.1", "--resume", str(tmp_path / "latest.pt")], check=True,
                   capture_output=True, cwd=HERE)
    first = json.loads((tmp_path / "metrics.jsonl").read_text().splitlines()[len(lines)])
    assert first["update"] == saved["update"] + 1
    assert first["noise_measured"] == saved["noise_scale"]["measured"] + 1
    assert first["lr_b_ref"] == round(saved["lr_rule"]["b_ref"])
    factor = saved["lr_rule"]["factor"]
    assert factor / 1.5 - 1e-5 <= first["lr_factor"] <= min(1.0, factor * 1.5) + 1e-5  # logged to 5 digits
    # The lr of the first resumed update used the saved factor (schedule: 3e-4 falling to 0.3x over 10 minutes).
    assert first["lr"] == pytest.approx(3e-4 * (1 - 0.7 * first["progress"]) * factor, rel=1e-6)
