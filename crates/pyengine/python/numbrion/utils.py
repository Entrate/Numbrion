"""Small numpy helpers around the masks (sampling, for tests and baselines)."""

from __future__ import annotations

import numpy as np


def sample_masked(mask: np.ndarray, rng: np.random.Generator) -> np.ndarray:
    """Uniformly sample one True index along the last axis of a boolean ``mask``.

    Rows without a True entry yield -1. Works for any leading shape.
    """
    mask = np.asarray(mask, dtype=bool)
    noise = rng.random(mask.shape)
    noise = np.where(mask, noise, -1.0)
    idx = noise.argmax(axis=-1).astype(np.int32)
    return np.where(mask.any(axis=-1), idx, -1).astype(np.int32)


def sample_uniform(result: dict, env, rng: np.random.Generator) -> np.ndarray:
    """Uniform two-stage action sampling for a ``BatchEnv`` result: slot 0 from ``mask0``, then slot 1 from the
    mask conditioned on it. Returns int32 ``[n_envs, 2, 2]``; sides that need no action get -1.

    Every code is equally likely, which is a much more tera- and switch-happy policy than a sensible one;
    it is meant for tests and random baselines.
    """
    a0 = sample_masked(result["mask0"], rng)  # [n, 2]
    m1 = env.mask_slot1(a0)  # [n, 2, A]
    a1 = sample_masked(m1, rng)
    out = np.stack([a0, a1], axis=-1).astype(np.int32)
    out[~result["needs_action"]] = -1
    return out


def grid_actions(p1_pairs, p2_pairs) -> np.ndarray:
    """All joint cells of a search grid: ``[k1, 2]`` slot-code pairs of side 0 and ``[k2, 2]`` of side 1 ->
    int32 ``[k1 * k2, 2, 2]`` for ``GridExecutor.run``, cell ``i * k2 + j`` = (``p1_pairs[i]``, ``p2_pairs[j]``).

    A side that needs no action at the root may pass ``[[-1, -1]]``.
    """
    p1 = np.asarray(p1_pairs, dtype=np.int32).reshape(-1, 2)
    p2 = np.asarray(p2_pairs, dtype=np.int32).reshape(-1, 2)
    out = np.empty((len(p1), len(p2), 2, 2), dtype=np.int32)
    out[:, :, 0] = p1[:, None]
    out[:, :, 1] = p2[None, :]
    return out.reshape(-1, 2, 2)
