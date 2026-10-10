"""PPO pieces that work on DirectML: masked sampling, GAE, a plain Adam and gradient clipping, plus the adaptive
entropy coefficient and the potential-based KO shaping of the self-play trainer."""
from __future__ import annotations

import math

import numpy as np
import torch

PASS = 46


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

    @torch.no_grad()
    def step(self) -> None:
        self.step_count += 1
        b1, b2 = self.betas
        correction1 = 1 - b1 ** self.step_count
        correction2 = 1 - b2 ** self.step_count
        device = self.parameters[0].device
        inverse2 = torch.full((), 1 / correction2, device=device)
        step_size = torch.full((), -self.lr / correction1, device=device)
        for p, m, v in zip(self.parameters, self.m, self.v):
            if p.grad is None:
                continue
            m.mul_(b1).add_(p.grad * (1 - b1))
            v.mul_(b2).add_(p.grad.square().mul_(1 - b2))
            denominator = (v * inverse2).sqrt_().add_(self.eps)
            p.add_(m.div(denominator).mul_(step_size))

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


def clip_gradients(parameters, max_norm: float) -> float:
    """Global-norm clipping from a scalar reduction (DirectML lacks the fused foreach/linalg kernels)."""
    grads = [p.grad for p in parameters if p.grad is not None]
    norm = float(sum(g.square().sum() for g in grads).sqrt().cpu())
    if not np.isfinite(norm):
        raise RuntimeError("Non-finite gradient norm")
    if norm > max_norm:
        for g in grads:
            g.mul_(max_norm / (norm + 1e-6))
    return norm


class EntropyController:
    """Entropy-bonus coefficient that keeps the policy entropy near a target schedule (an exploration floor).

    The entropy is the batch mean of the slot-a plus slot-b masked policy entropies, as logged by the trainer
    (forced decisions such as PASS count as zero). The target falls linearly from ``start`` to ``final`` over the
    run. After every update, ``log(coef)`` moves by ``rate * (target - entropy)`` (the error clipped to +-1 nat), so
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
