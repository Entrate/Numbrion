"""PPO pieces that work on DirectML: masked sampling, GAE, a plain Adam and gradient clipping."""
from __future__ import annotations

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
