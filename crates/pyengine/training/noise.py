"""Gradient noise scale (McCandlish et al. 2018, "An Empirical Model of Large-Batch Training", appendix A) and the
learning-rate rule driven by it (docs/training/SELFPLAY.md, "Noise scale and automatic learning rate").

With ``m`` gradients ``g_i`` of small batches of ``b`` rows and their mean ``G`` (``B = m b`` rows):

    |G_true|^2 ~ (B |G|^2 - b mean|g_i|^2) / (B - b),   tr(Sigma) ~ (mean|g_i|^2 - |G|^2) / (1/b - 1/B),

and ``B_noise = tr(Sigma) / |G_true|^2``, the batch size (rows) at which the gradient's noise equals its signal. Both
estimates are unbiased for independent rows; their ratio is not: smooth each over updates and divide the smoothed
values (``NoiseScale``).

The trainer's probe (train_selfplay.ppo_update) measures one rollout per measured update, before the update's first
step, at the weight average (``--noise-weights``): at the raw training weights the signal grows with the learning
rate (the weights jitter around the mean point and the true gradient there is larger), which would feed back into a
learning-rate rule. Its small batches are whole envs (``env_chunks``): a battle's decisions share its outcome and
self-play puts both sides in one env, so random rows could hide that shared noise from ``tr(Sigma)`` and push it into
the one-rollout ``|G_true|^2``. Pairing the mean gradients of two updates does not work: the update between them
stepped along the first one's noise, and the true gradient at the new weights points partly back (adjacent updates
gave a negative signal). Measurements: docs/training/SELFPLAY.md.

* ``GradientNoise`` collects one update's small-batch gradients on the device (no CPU sync until ``estimate``).
* ``env_chunks`` cuts a rollout's trained rows into equal chunks that keep each env's rows together.
* ``NoiseScale`` smooths the per-update estimates with bias-corrected EMAs (like Adam's moments).
* ``LrRule`` turns the smoothed ``B_noise`` into a step-temperature factor: ``f = clamp((B_ref / B_noise) ** power,
  min_factor, 1)``, moving at most ``max_change`` (relative) per update. ``mode`` says how the factor is applied:
  ``lr`` multiplies the scheduled learning rate, ``accumulate`` keeps it and sums ``round(1 / f)`` minibatch gradients
  per optimizer step (a bigger batch instead of a smaller step). Another rule only needs ``update``, ``lr_factor``,
  ``accumulate`` and the state methods.
"""
from __future__ import annotations

import math

import numpy as np
import torch


def noise_estimates(mean_square: float, mean_grad_square: float, small: float, big: float) -> tuple[float, float]:
    """``(|G_true|^2, tr(Sigma))`` from the mean squared norm of small-batch gradients (``small`` rows each) and the
    squared norm of their mean (``big`` rows in all)."""
    if not big > small > 0:
        raise ValueError("noise_estimates needs big > small > 0")
    g2 = (big * mean_grad_square - small * mean_square) / (big - small)
    trace = (mean_square - mean_grad_square) / (1 / small - 1 / big)
    return g2, trace


class GradientNoise:
    """One update's small-batch gradients: their sum and summed squared norms stay on the device until ``estimate``.

    Every gradient must come from a batch of the same size, and from the same parameters (``None`` entries, the
    parameters a loss does not reach, are skipped and must be the same every time)."""

    def __init__(self):
        self.total: torch.Tensor | None = None
        self.square: torch.Tensor | None = None
        self.count = 0
        self.rows = None

    def add(self, grads, rows: int) -> None:
        if self.rows is not None and rows != self.rows:
            raise ValueError("GradientNoise needs equal batch sizes")
        flat = torch.cat([g.detach().reshape(-1) for g in grads if g is not None])
        if self.total is not None and flat.numel() != self.total.numel():
            raise ValueError("GradientNoise got gradients of other parameters")
        square = flat.square().sum()
        self.total = flat.clone() if self.total is None else self.total.add_(flat)
        self.square = square if self.square is None else self.square + square
        self.count += 1
        self.rows = rows

    def estimate(self) -> dict | None:
        """``g2`` (|G_true|^2), ``trace`` (tr(Sigma)), ``b_noise`` (their ratio, ``None`` unless ``g2`` > 0) and the
        batch sizes; ``None`` below two gradients."""
        if self.count < 2:
            return None
        mean_square = float(self.square) / self.count
        # No tensor op with a Python scalar: DirectML keeps host memory for each (training.ppo.Adam).
        mean_grad_square = float(self.total.square().sum()) / self.count ** 2
        small, big = self.rows, self.rows * self.count
        g2, trace = noise_estimates(mean_square, mean_grad_square, small, big)
        return dict(g2=g2, trace=trace, b_noise=trace / g2 if g2 > 0 else None, small=small, big=big,
                    gradients=self.count)


def env_chunks(selected: np.ndarray, rows_per_step: int, size: int) -> list[np.ndarray]:
    """Positions into ``selected`` (flat ``step * rows_per_step + row`` indices of a rollout's trained rows) in chunks
    of exactly ``size``, ordered by env (rows ``2e`` and ``2e + 1``, both sides of a battle) and then step, so a chunk
    holds whole envs except where a cut splits one; the remainder is dropped. Equal sizes keep DirectML to one
    compiled shape."""
    env = (selected % rows_per_step) // 2
    order = np.lexsort((selected, env))  # by env, then by flat index (step-major, so by step)
    return [order[k * size:(k + 1) * size] for k in range(len(order) // size)]


class NoiseScale:
    """Bias-corrected EMAs (time constant ``updates`` measurements) of ``|G_true|^2`` and ``tr(Sigma)``; ``b_noise``
    is their ratio (``inf`` while the smoothed signal is not positive, ``None`` before the first measurement).
    ``every``: measure every that many updates (``due``)."""

    def __init__(self, updates: float = 50.0, every: int = 4):
        if updates < 1 or every < 1:
            raise ValueError("NoiseScale needs updates >= 1 and every >= 1")
        self.decay = 1.0 - 1.0 / updates
        self.every = int(every)
        self.g2 = self.trace = self.weight = 0.0
        self.measured = 0

    def due(self, update: int) -> bool:
        return update % self.every == 0

    def observe(self, g2: float, trace: float) -> None:
        if not (math.isfinite(g2) and math.isfinite(trace)):
            return
        d = self.decay
        self.g2 = d * self.g2 + (1 - d) * g2
        self.trace = d * self.trace + (1 - d) * trace
        self.weight = d * self.weight + (1 - d)
        self.measured += 1

    @property
    def b_noise(self) -> float | None:
        if not self.measured:
            return None
        return self.trace / self.g2 if self.g2 > 0 else math.inf

    def summary(self) -> dict:
        """Smoothed values for the metrics line (bias-corrected)."""
        if not self.measured:
            return dict(noise_b=None, noise_g2=None, noise_trace=None, noise_measured=0)
        b = self.b_noise
        return dict(noise_b=round(b) if math.isfinite(b) else None, noise_g2=float(f"{self.g2 / self.weight:.4g}"),
                    noise_trace=float(f"{self.trace / self.weight:.4g}"), noise_measured=self.measured)

    def state_dict(self) -> dict:
        return dict(g2=self.g2, trace=self.trace, weight=self.weight, measured=self.measured)

    def load_state_dict(self, state: dict) -> None:
        self.g2, self.trace, self.weight = float(state["g2"]), float(state["trace"]), float(state["weight"])
        self.measured = int(state["measured"])


class LrRule:
    """Step-temperature factor from the smoothed noise scale.

    ``B_ref`` is given, or taken as the smoothed ``B_noise`` once ``warmup`` updates have been measured (and it is
    finite); until then the factor stays 1. After that, every measured update moves the factor toward
    ``clamp((B_ref / B_noise) ** power, min_factor, 1)`` by at most a factor ``1 + max_change`` either way (a
    non-positive smoothed signal counts as ``B_noise = inf``: the target is ``min_factor``).

    ``mode="lr"``: the learning rate is the scheduled one times ``lr_factor``. ``mode="accumulate"``: the learning
    rate keeps the schedule and each optimizer step sums ``accumulate = round(1 / factor)`` minibatch gradients.
    """

    MODES = ("lr", "accumulate")

    def __init__(self, power: float = 0.5, min_factor: float = 0.1, max_change: float = 0.02,
                 b_ref: float | None = None, warmup: int = 50, mode: str = "lr"):
        if mode not in self.MODES:
            raise ValueError(f"LrRule mode must be one of {self.MODES}")
        if not (power > 0 and 0 < min_factor <= 1 and max_change > 0 and warmup >= 1):
            raise ValueError("LrRule needs power > 0, 0 < min_factor <= 1, max_change > 0 and warmup >= 1")
        if b_ref is not None and not (b_ref > 0 and math.isfinite(b_ref)):
            raise ValueError("LrRule b_ref must be positive and finite")
        self.power, self.min_factor, self.max_change = power, min_factor, max_change
        self.b_ref, self.warmup, self.mode = b_ref, int(warmup), mode
        self.factor = 1.0

    def target(self, b_noise: float) -> float:
        if b_noise is None or self.b_ref is None:
            return self.factor
        if not b_noise > 0:  # a negative trace estimate: nothing to say
            return self.factor
        raw = 0.0 if math.isinf(b_noise) else (self.b_ref / b_noise) ** self.power
        return min(max(raw, self.min_factor), 1.0)

    def update(self, noise: NoiseScale) -> float:
        """After a measured update: fix ``B_ref`` after the warm-up, then step the factor; returns it."""
        b = noise.b_noise
        if self.b_ref is None:
            if noise.measured >= self.warmup and b is not None and math.isfinite(b) and b > 0:
                self.b_ref = b
            return self.factor
        step = 1.0 + self.max_change
        self.factor = min(max(self.target(b), self.factor / step), self.factor * step)
        self.factor = min(max(self.factor, self.min_factor), 1.0)
        return self.factor

    @property
    def lr_factor(self) -> float:
        return self.factor if self.mode == "lr" else 1.0

    @property
    def accumulate(self) -> int:
        return max(1, round(1.0 / self.factor)) if self.mode == "accumulate" else 1

    def summary(self) -> dict:
        return dict(lr_factor=round(self.factor, 5), lr_b_ref=None if self.b_ref is None else round(self.b_ref),
                    accumulate=self.accumulate)

    def state_dict(self) -> dict:
        return dict(factor=self.factor, b_ref=self.b_ref, mode=self.mode)

    def load_state_dict(self, state: dict, keep_b_ref: bool = False) -> None:
        """Restore the factor and ``B_ref`` (``keep_b_ref``: keep this rule's given ``B_ref``). A saved factor from the
        other mode is ignored: its meaning differs."""
        if state.get("mode", "lr") == self.mode:
            self.factor = min(max(float(state["factor"]), self.min_factor), 1.0)
        if not keep_b_ref:
            self.b_ref = state["b_ref"]
