"""PPO pieces that work on DirectML: masked sampling, GAE, a plain Adam and gradient clipping, plus the adaptive
entropy coefficient and the potential-based KO shaping of the self-play trainer."""
from __future__ import annotations

import math

import numpy as np
import torch

PASS = 46


def tera_choice_entropy(logprobs: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
    """Return base entropy and expected conditional Tera entropy per slot, shaped ``[...]``.

    The joint masked policy is ``[..., 47]``: codes 0..39 are 20 adjacent (no Tera, Tera) pairs;
    codes 40..46 are six switches and PASS. Logsumexp merges each pair into its base choice,
    giving 27 options. The Tera term is E_b[H(Tera | b)], weighted by the move pair's joint
    probabilities; switches/PASS contribute zero. Their sum is the original joint entropy.

    Replace -inf masks with the model's finite sentinel before reducing: an entirely illegal
    pair otherwise has undefined logsumexp gradients and 0 * -inf terms. Its probability still
    underflows to zero. Reshapes/slices and reductions avoid DirectML's int64 indexing fallback.
    """
    lp = logprobs.clamp_min(-1e9)
    moves = lp[..., :40].reshape(*lp.shape[:-1], 20, 2)
    move_base = torch.logsumexp(moves, dim=-1)
    base = torch.cat((move_base, lp[..., 40:]), dim=-1)
    base_entropy = -(base.exp() * base).sum(-1)
    tera_entropy = -(moves.exp() * (moves - move_base.unsqueeze(-1))).sum(-1).sum(-1)
    return base_entropy, tera_entropy


def sample(logprobs: np.ndarray, rng: np.random.Generator) -> np.ndarray:
    """Gumbel-max sampling from masked log-probabilities ([rows, 47]; illegal codes are ~-1e9)."""
    return np.argmax(logprobs + rng.gumbel(size=logprobs.shape), axis=-1).astype(np.int32)


def gae(rewards, values, dones, bootstrap, gamma: float, lam: float):
    """Generalized advantage estimation over [T, rows]; ``dones[t]`` ends the episode after step t."""
    advantages = np.zeros_like(rewards)
    carry = np.zeros_like(bootstrap)
    for t in reversed(range(len(rewards))):
        following = bootstrap if t == len(rewards) - 1 else values[t + 1]
        live = 1.0 - dones[t].astype(np.float32)
        delta = rewards[t] + gamma * following * live - values[t]
        carry = delta + gamma * lam * live * carry
        advantages[t] = carry
    return advantages, advantages + values


def minibatches(order: np.ndarray, size: int):
    """``order`` in minibatches of ``size``; the last one is cut to a multiple of ``size // 8`` and dropped below
    ``size // 4``.

    DirectML compiles kernels for every new input shape and never frees them (~4.4 MB per minibatch size). The
    trained-row count changes every rollout, so a last minibatch of any length added a new shape almost every update
    (~3.8 MB per update, measured); this way a run sees at most seven sizes.
    """
    granule = max(size // 8, 1)
    for begin in range(0, len(order), size):
        pick = order[begin:begin + size]
        pick = pick[:len(pick) - len(pick) % granule]
        if len(pick) >= size // 4:
            yield pick


class Adam:
    """Adam from elementwise ops; torch's Adam uses ``lerp``, which DirectML runs on the CPU.

    No ``alpha=``/``value=`` arguments and no per-step Python scalars: with those (``addcdiv_(..., value=-lr / c1)``,
    ``add_(grad, alpha=1 - b1)``, ``v / c2``) DirectML kept host memory on every call, ~3.6 MB per PPO update. The
    bias corrections and step size are 0-dim device tensors instead (0.1 MB per update, measured).
    """

    def __init__(self, parameters, lr: float, betas=(0.9, 0.999), eps: float = 1e-5):
        self.parameters = [p for p in parameters if p.requires_grad]
        self.lr, self.betas, self.eps = lr, betas, eps
        self.step_count = 0
        self.m = [torch.zeros_like(p) for p in self.parameters]
        self.v = [torch.zeros_like(p) for p in self.parameters]
        self._constants: dict = {}

    @torch.no_grad()
    def step(self, skip: torch.Tensor | None = None) -> None:
        """One Adam step. ``skip`` (a 0-dim bool device tensor, e.g. ``~torch.isfinite(norm)`` of the norm
        ``clip_gradients`` returns) makes it a no-op on the device, without a CPU sync: the weights and moments keep
        their values whatever the gradients hold, where a NaN gradient would turn all three into NaN. Only the step
        counter advances."""
        self.step_count += 1
        b1, b2 = self.betas
        correction1 = 1 - b1 ** self.step_count
        correction2 = 1 - b2 ** self.step_count
        device = self.parameters[0].device
        dtype = self.parameters[0].dtype  # DirectML's torch.full defaults to float64 without an explicit dtype.
        if device.type == "mps":
            self._step_fused(skip)
            return
        inverse2 = torch.full((), 1 / correction2, device=device, dtype=dtype)
        step_size = torch.full((), -self.lr / correction1, device=device, dtype=dtype)
        decay1, decay2 = b1, b2
        if skip is not None:  # skipped: zero gradient, no decay, zero step (cached constants, see the class doc)
            if device not in self._constants:
                self._constants[device] = tuple(torch.full((), x, device=device, dtype=dtype)
                                                for x in (1.0, 0.0, b1, b2))
            one, zero, beta1, beta2 = self._constants[device]
            decay1, decay2 = torch.where(skip, one, beta1), torch.where(skip, one, beta2)
            step_size = torch.where(skip, zero, step_size)
        for p, m, v in zip(self.parameters, self.m, self.v):
            if p.grad is None:
                continue
            grad = p.grad if skip is None else torch.where(skip, zero, p.grad)
            m.mul_(decay1).add_(grad * (1 - b1))
            v.mul_(decay2).add_(grad.square().mul_(1 - b2))
            denominator = (v * inverse2).sqrt_().add_(self.eps)
            p.add_(m.div(denominator).mul_(step_size))

    def _step_fused(self, skip: torch.Tensor | None) -> None:
        """torch's fused Adam kernel on the same state: 1.4 ms per step on MPS against 4.8 ms for the loop (six
        launches per parameter tensor). Same update up to rounding: it forms ``m`` with ``lerp`` and divides by
        ``sqrt(c2)`` after the square root."""
        live = [i for i, p in enumerate(self.parameters) if p.grad is not None]
        if not live:  # the kernel rejects empty lists; the loop would update nothing
            return
        step = torch.full((), float(self.step_count), device=self.parameters[0].device)
        torch._fused_adam_([self.parameters[i] for i in live], [self.parameters[i].grad for i in live],
                           [self.m[i] for i in live], [self.v[i] for i in live], [], [step] * len(live),
                           amsgrad=False, lr=self.lr, beta1=self.betas[0], beta2=self.betas[1], weight_decay=0.0,
                           eps=self.eps, maximize=False, found_inf=None if skip is None else skip.float())

    def zero_grad(self) -> None:
        for p in self.parameters:
            p.grad = None

    def state_dict(self) -> dict:
        return {"step": self.step_count, "m": [x.detach().cpu() for x in self.m], "v": [x.detach().cpu() for x in self.v]}

    def load_state_dict(self, state: dict) -> None:
        self.step_count = state["step"]
        for target, source in zip(self.m, state["m"]):
            target.copy_(source.to(target.device))
        for target, source in zip(self.v, state["v"]):
            target.copy_(source.to(target.device))


_CLIP_CONSTANTS: dict = {}


def clip_gradients(parameters, max_norm: float) -> torch.Tensor:
    """Global-norm clipping from a scalar reduction (DirectML lacks the fused foreach/linalg kernels).

    Stays on the device (no CPU sync per minibatch; the caller reads the returned norms once per update). A
    non-finite norm (an inf or NaN gradient, or a sum that overflows) leaves the gradients unusable: pass
    ``skip=~torch.isfinite(norm)`` to ``Adam.step`` so that step changes nothing, and raise afterwards (the trainer
    does, once per update). Constants are cached device tensors: DirectML keeps host memory for every op given a
    Python scalar (see Adam).
    """
    grads = [p.grad for p in parameters if p.grad is not None]
    if grads[0].device.type == "mps":  # one norm over all gradients: 0.9 ms with the rescale, the loop 2.3 ms
        norm = torch.cat([g.reshape(-1) for g in grads]).square().sum().sqrt()
    else:
        norm = torch.stack([g.square().sum() for g in grads]).sum().sqrt()
    key = (norm.device, norm.dtype, float(max_norm))
    if key not in _CLIP_CONSTANTS:
        _CLIP_CONSTANTS[key] = tuple(torch.full((), x, device=norm.device, dtype=norm.dtype)
                                     for x in (max_norm, 1e-6, 1.0, 0.0))
    limit, eps, one, zero = _CLIP_CONSTANTS[key]
    scale = torch.minimum(one, limit / (norm + eps))
    scale = torch.where(torch.isfinite(norm), scale, zero)
    if norm.device.type == "mps":
        torch._foreach_mul_(grads, scale)
    else:
        for g in grads:
            g.mul_(scale)
    return norm.detach()


class WeightAverage:
    """Exponential moving average of a model's weights, ``avg <- decay * avg + (1 - decay) * w`` after every update
    (``decay = 1 - 1 / updates``), started from the model's current weights.

    Late in training the PPO snapshots are noisy points around a better policy: on 2026-10-15 the plain average of 28
    snapshots (about 9 hours) beat every single one of them by 3-5 points head to head. Kept on the model's device;
    the mixing constants are cached device tensors (DirectML keeps host memory for every op given a Python scalar, see
    Adam). Integer entries are copied, not averaged."""

    def __init__(self, model: torch.nn.Module, updates: float):
        if updates < 1:
            raise ValueError("WeightAverage needs updates >= 1")
        self.decay = 1.0 - 1.0 / updates
        self.state = {k: v.detach().clone() for k, v in model.state_dict().items()}
        self._constants: dict = {}

    @torch.no_grad()
    def update(self, model: torch.nn.Module) -> None:
        for key, value in model.state_dict().items():
            average = self.state[key]
            if not average.is_floating_point():
                average.copy_(value)
                continue
            constants = (average.device, average.dtype)
            if constants not in self._constants:
                self._constants[constants] = tuple(torch.full((), x, device=average.device, dtype=average.dtype)
                                                   for x in (self.decay, 1.0 - self.decay))
            keep, mix = self._constants[constants]
            average.mul_(keep).add_(value * mix)

    def state_dict(self) -> dict:
        return {k: v.detach().cpu().clone() for k, v in self.state.items()}

    def load_state_dict(self, state: dict) -> None:
        for key, value in state.items():
            self.state[key].copy_(value.to(self.state[key].device))


class EntropyController:
    """Entropy-bonus coefficient that keeps the policy entropy near a target schedule (an exploration floor).

    The entropy is the batch mean of the slot-a plus slot-b base-choice entropies, with the Tera bit marginalized
    out, as logged by the trainer (forced decisions such as PASS count as zero). The target falls linearly from
    ``start`` to ``final`` over the run. After every update, ``log(coef)`` moves by ``rate * (target - entropy)``
    (the error clipped to +-1 nat), so
    the coefficient grows by up to ~22% per update while the entropy is below target and shrinks while it is above;
    it stays within ``[low, high]``.
    """

    def __init__(self, coef: float, start: float, final: float, rate: float = 0.2, low: float = 1e-3,
                 high: float = 0.05):
        self.start, self.final, self.rate = start, final, rate
        self.low, self.high = low, high
        self.log_coef = math.log(min(max(coef, low), high))

    @property
    def coef(self) -> float:
        return math.exp(self.log_coef)

    def target(self, progress: float) -> float:
        return self.start + (self.final - self.start) * min(max(progress, 0.0), 1.0)

    def update(self, entropy: float, progress: float) -> float:
        """Adapt to the entropy measured with the current coefficient; returns the coefficient for the next update."""
        if np.isfinite(entropy):
            error = min(max(self.target(progress) - entropy, -1.0), 1.0)
            self.log_coef = min(max(self.log_coef + self.rate * error, math.log(self.low)), math.log(self.high))
        return self.coef

    def state_dict(self) -> dict:
        return {"log_coef": self.log_coef}

    def load_state_dict(self, state: dict) -> None:
        self.log_coef = min(max(float(state["log_coef"]), math.log(self.low)), math.log(self.high))


def ko_shaping(potential: np.ndarray, fainted_after: np.ndarray, done: np.ndarray,
               bonus: float) -> tuple[np.ndarray, np.ndarray]:
    """Potential-based KO shaping per row: ``Phi(after) - Phi(before)`` with ``Phi = bonus * (foe KOs - own KOs)``.

    ``potential`` [rows] is each row's stored ``Phi`` from its previous step (zero at a battle's start);
    ``fainted_after`` is [rows, 2] (own, foe) KO counts; ``done`` [rows] marks rows whose battle just ended, where the
    potential is zero. Returns (shaping reward, new potential). Subtracting the stored potential, rather than one
    recomputed with the current bonus, keeps the bonus summing to exactly zero over a battle even while it decays
    between rollouts, and settles what is owed once it reaches zero. It only moves credit for KOs earlier and no
    longer pays for the KO margin of a win or loss (the old bonus did, which favoured all-out trading). gamma is
    taken as 1 (0.995 in PPO; the difference is below 1e-3 per step).
    """
    after = np.where(done, 0.0, bonus * (fainted_after[:, 1] - fainted_after[:, 0])).astype(np.float32)
    return (after - potential).astype(np.float32), after
