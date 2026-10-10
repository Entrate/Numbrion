"""Scripted players shared by evaluation (evaluate.py) and training opponents (training.actors).

* ``RandomAgent``: a random legal player (switch 10%, Tera 15%, otherwise a uniform legal move).
* ``HeuristicAgent``: "always use the strongest attack" from the damage-calc features, never Tera or a voluntary
  switch. Predictable on purpose: as a training opponent it rewards Protect, switching and support moves.

Both use the two-call interface of the evaluation loop: ``first(arrays, rows, mask0)`` picks slot-a codes for the
given observation rows, ``second(rows, mask1)`` picks slot-b codes with the conditional slot-b masks. Only legal
codes are returned (rows whose mask is all-false get PASS).
"""
from __future__ import annotations

import numpy as np

N_ACTIONS = 47
PASS = 46


def legal_random(mask: np.ndarray, rng, switch_prob=0.1, tera_prob=0.15) -> np.ndarray:
    """Per row: a switch with ``switch_prob`` if one is legal, else a uniform legal non-Tera move code (Tera with
    ``tera_prob`` when legal), else any legal code."""
    out = np.full(len(mask), PASS, dtype=np.int32)
    codes = np.arange(N_ACTIONS)
    for i, m in enumerate(mask):
        moves = codes[:40][m[:40]]
        switches = codes[40:46][m[40:46]]
        plain = moves[moves % 2 == 0]
        tera = moves[moves % 2 == 1]
        if len(switches) and (rng.random() < switch_prob or not len(moves)):
            out[i] = rng.choice(switches)
        elif len(tera) and rng.random() < tera_prob:
            out[i] = rng.choice(tera)
        elif len(plain):
            out[i] = rng.choice(plain)
        elif m.any():
            out[i] = rng.choice(codes[m])
    return out


def forced_logprobs(actions: np.ndarray) -> np.ndarray:
    """[rows, 47] log-probabilities putting all mass on ``actions`` (so masked Gumbel sampling returns them)."""
    out = np.full((len(actions), N_ACTIONS), -1e9, dtype=np.float32)
    out[np.arange(len(actions)), actions] = 0.0
    return out


class RandomAgent:
    name = "random"

    def __init__(self, rng):
        self.rng = rng

    def first(self, arrays, rows, mask0):
        return legal_random(mask0, self.rng)

    def second(self, rows, mask1):
        return legal_random(mask1, self.rng)


class HeuristicAgent:
    """Always the strongest attack: highest expected damage to foes minus damage to its own side, no Tera."""

    name = "heuristic"

    def __init__(self, rng):
        self.rng = rng

    def _pick(self, mask, features, slot):
        out = legal_random(mask, self.rng, switch_prob=0.0, tera_prob=0.0)
        score = features[:, slot, :, 0] - features[:, slot, :, 1]  # [rows, 40]
        legal = mask[:, :40] & (np.arange(40) % 2 == 0)
        score = np.where(legal, score, -np.inf)
        best = score.argmax(-1)
        good = np.isfinite(score.max(-1)) & (score.max(-1) > 0)
        return np.where(good, best, out).astype(np.int32)

    def first(self, arrays, rows, mask0):
        self.features = arrays["action_features"][rows]
        return self._pick(mask0, self.features, 0)

    def second(self, rows, mask1):
        return self._pick(mask1, self.features, 1)
