"""The entropy regularizer treats Tera as a conditional choice; PPO still uses joint codes."""
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

from train_selfplay import ppo_loss
from training.diagnostics import ActionUsage
from training.dex import load_dex
from training.encoder import FF, I_MOVE0, N_FIELD_FLOATS, N_IDS
from training.ppo import Adam, clip_gradients, tera_choice_entropy


@pytest.mark.parametrize("masked", [-1e9, -float("inf")])
@pytest.mark.parametrize("dtype", [torch.float32, torch.float64])
def test_entropy_chain_rule_with_masked_and_forced_choices(masked, dtype):
    generator = torch.Generator().manual_seed(73)
    logits = torch.randn(2, 8, 47, generator=generator, dtype=dtype, requires_grad=True)
    mask = torch.rand(2, 8, 47, generator=generator) > 0.4
    mask[..., 46] = True
    mask[:, 0, 1:40:2] = False  # Tera is unavailable.
    mask[:, 1, :40] = False  # Only switches and PASS.
    mask[:, 2] = False
    mask[:, 2, 46] = True  # Forced PASS.
    mask[:, 3] = False
    mask[:, 3, 41] = True  # Forced switch.
    mask[:, 4] = False
    mask[:, 4, 1] = True  # Only a Tera move variant is legal.
    lp = logits.masked_fill(~mask, masked).log_softmax(-1)
    base, tera = tera_choice_entropy(lp)
    # Zero out the masked log-probs to use the convention 0 log 0 = 0.
    joint = -(lp.exp() * torch.where(mask, lp, torch.zeros_like(lp))).sum(-1)
    torch.testing.assert_close(base + tera, joint)
    torch.testing.assert_close(tera[:, :4], torch.zeros_like(tera[:, :4]))
    torch.testing.assert_close(base[:, 2:5], torch.zeros_like(base[:, 2:5]))
    assert (tera >= -1e-7).all()
    split_grad = torch.autograd.grad((base + tera).sum(), logits, retain_graph=True)[0]
    joint_grad = torch.autograd.grad(joint.sum(), logits)[0]
    assert torch.isfinite(split_grad).all()
    torch.testing.assert_close(split_grad, joint_grad)


def factorized_logprobs(difference):
    """Vary the Tera logit difference while keeping all 27 base probabilities fixed."""
    base = torch.linspace(-1, 1, 27, dtype=difference.dtype).log_softmax(-1)
    bit_logits = torch.stack((-difference / 2, difference / 2), -1)
    move_lp = base[:20, None] + bit_logits.log_softmax(-1)
    return torch.cat((move_lp.reshape(40), base[20:])).unsqueeze(0)


def test_zero_weight_has_no_conditional_tera_entropy_gradient():
    difference = torch.linspace(-2, 2, 20, dtype=torch.float64, requires_grad=True)
    base, tera = tera_choice_entropy(factorized_logprobs(difference))
    bonus = (base + 0.0 * tera).sum()
    gradient = torch.autograd.grad(bonus, difference, retain_graph=True)[0]
    torch.testing.assert_close(gradient, torch.zeros_like(gradient), atol=1e-14, rtol=0)
    # With the old joint bonus the split is encouraged toward 50/50.
    old_gradient = torch.autograd.grad((base + tera).sum(), difference)[0]
    assert (old_gradient * difference < 0).all()


def test_directml_entropy_backward_and_optimizer_use_float32():
    if sys.platform != "win32":
        pytest.skip("DirectML requires Windows")
    directml = pytest.importorskip("torch_directml")
    if not directml.device_count():
        pytest.skip("DirectML adapter unavailable")
    # This torch build cannot add DirectML autograd queues after CPU backward has initialized
    # the engine. A fresh process also matches the trainer's device initialization order.
    result = subprocess.run(
        [sys.executable, "-c", "import runpy; "
         "runpy.run_path('training_tests/test_tera_choice.py')['_check_directml_entropy_optimizer']()"],
        cwd=Path(__file__).resolve().parents[1], capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def _check_directml_entropy_optimizer():
    import torch_directml as directml

    device = directml.device()
    generator = torch.Generator().manual_seed(73)
    logits = torch.randn(4, 47, generator=generator).to(device).requires_grad_()
    mask = torch.ones(4, 47, dtype=torch.bool)
    mask[0, 1:40:2] = False
    mask[1, :40] = False
    mask[2] = False
    mask[2, 46] = True
    lp = logits.masked_fill(~mask.to(device), -1e9).log_softmax(-1)
    base, tera = tera_choice_entropy(lp)
    joint = -(lp.exp() * lp).sum(-1)
    torch.testing.assert_close((base + tera).cpu(), joint.cpu())
    base.mean().backward()
    before = logits.detach().cpu().clone()
    optimizer = Adam([logits], lr=0.01)
    norm = clip_gradients(optimizer.parameters, 0.5)
    assert norm.dtype == torch.float32 and torch.isfinite(norm.cpu())
    optimizer.step(skip=~torch.isfinite(norm))
    assert logits.grad.dtype == torch.float32
    assert torch.isfinite(logits.detach().cpu()).all()
    assert not torch.equal(before, logits.detach().cpu())


@pytest.mark.parametrize("tera_weight", [0.0, 1.0])
def test_ppo_loss_uses_base_bonus_and_retains_joint_ratio(tera_weight):
    lp0 = factorized_logprobs(torch.linspace(-2, 2, 20, dtype=torch.float64))
    lp1 = factorized_logprobs(torch.linspace(1, 3, 20, dtype=torch.float64))
    zero = lp0.new_zeros(1)

    class Policy:
        def trunk(self, *args):
            return None

        def slot0(self, *args):
            return lp0, None

        def slot1(self, *args):
            return lp1

        def value(self, *args):
            return zero

        def aux_loss(self, *args):
            return zero.mean()

    batch = dict.fromkeys(("ids", "floats", "field", "action_features", "token_features", "foe_match"))
    a0, a1 = torch.tensor([1]), torch.tensor([8])
    joint_logp = lp0[:, 1] + lp1[:, 8]
    ratio = 1.1
    old_logp = joint_logp - np.log(ratio)
    loss, terms = ppo_loss(Policy(), batch, None, None, a0, a1, None, None, old_logp,
                           torch.ones(1), zero, 0.0, 0.3, 0.0, 0.2, tera_weight)
    base0, tera0 = tera_choice_entropy(lp0)
    base1, tera1 = tera_choice_entropy(lp1)
    entropy, tera_entropy = (base0 + base1).mean(), (tera0 + tera1).mean()
    torch.testing.assert_close(terms[2], entropy)
    torch.testing.assert_close(terms[3], tera_entropy)
    assert float(terms[0]) == pytest.approx(-ratio)
    assert float(terms[5]) == pytest.approx(ratio - 1 - np.log(ratio))
    torch.testing.assert_close(loss, -ratio - 0.3 * (entropy + tera_weight * tera_entropy))


def test_action_usage_reports_tera_timing_for_learner_uses():
    steps, rows = 2, 3
    dex = load_dex()
    ids = np.zeros((steps, rows, 12, N_IDS), dtype=np.int64)
    ids[..., :2, I_MOVE0:I_MOVE0 + 4] = dex.move_index["thunderbolt"]
    field = np.zeros((steps, rows, N_FIELD_FLOATS), dtype=np.float32)
    field[..., FF["turn"]] = np.array([[1, 4, 1], [7, 10, 1]]) / 30
    mask = np.ones((steps, rows, 47), dtype=bool)
    data = dict(ids=ids, field=field, mask0=mask, mask1=mask,
                a0=np.array([[1, 0, 1], [0, 1, 1]]), a1=np.array([[0, 1, 1], [1, 0, 1]]),
                train=np.array([[True, True, False], [True, True, False]]))
    usage = ActionUsage(dex)
    metrics = usage(data)
    assert metrics["use_tera"] == 0.5
    assert metrics["tera_turn1"] == 0.25
    assert metrics["tera_mean_turn"] == 5.5
    # A use in slot b counts too; opponent rows and non-Tera moves do not.
    no_tera = usage(dict(data, a0=np.zeros((steps, rows), dtype=int), a1=np.zeros((steps, rows), dtype=int)))
    assert no_tera["tera_turn1"] is None and no_tera["tera_mean_turn"] is None
    assert usage(dict(data, train=np.zeros((steps, rows), dtype=bool))) == {}
