"""Opponents for self-play training: which side-2 player each battle gets, and the snapshot pool with
prioritized fictitious self-play (PFSP, AlphaStar).

Every env has a fixed opponent kind for the whole run, so a battle never changes kind halfway through:

* ``SELF``: the learner plays both sides and both are trained,
* ``SNAPSHOT``: a frozen older snapshot plays side 2 (one snapshot per rollout, chosen by PFSP),
* ``HEURISTIC``: the scripted "always use the strongest attack" player (training.scripted) plays side 2.

Only the learner's own decisions are trained on. Snapshot envs play self-play until the pool has two snapshots.
"""
from __future__ import annotations

import numpy as np

SELF, SNAPSHOT, HEURISTIC = 0, 1, 2
KIND_NAMES = {SELF: "self", SNAPSHOT: "snapshot", HEURISTIC: "heuristic"}


def spread_mask(n: int, fraction: float) -> np.ndarray:
    """``fraction`` of ``n`` entries set, evenly spread (so the opponent battles are shared across actors)."""
    index = np.arange(n)
    return np.floor((index + 1) * fraction) > np.floor(index * fraction)


def opponent_kinds(n_envs: int, pool_fraction: float, scripted_share: float) -> np.ndarray:
    """Per-env opponent kind: ``pool_fraction`` of the envs get a non-self opponent, ``scripted_share`` of those
    the heuristic and the rest a snapshot, both evenly spread over the envs (and so over the actors)."""
    kinds = np.full(n_envs, SELF, dtype=np.int8)
    pool = np.flatnonzero(spread_mask(n_envs, pool_fraction))
    scripted = spread_mask(len(pool), scripted_share)
    kinds[pool[scripted]] = HEURISTIC
    kinds[pool[~scripted]] = SNAPSHOT
    return kinds


def pfsp_weights(win_rates, power: float = 2.0, floor: float = 0.1) -> np.ndarray:
    """Sampling distribution over snapshots from the learner's win rates against them.

    ``(1 - win) ** power`` (AlphaStar's f_hard) favours the snapshots the learner still loses to; mixing in
    ``floor`` of a uniform distribution keeps every snapshot in play so beaten strategies are not forgotten.
    """
    win = np.clip(np.asarray(win_rates, dtype=np.float64), 0.0, 1.0)
    if not len(win):
        return win
    hard = (1.0 - win) ** power
    hard = hard / hard.sum() if hard.sum() > 0 else np.full(len(win), 1.0 / len(win))
    return (1.0 - floor) * hard + floor / len(win)


class SnapshotPool:
    """The last ``size`` learner snapshots with an EMA of the learner's score (win 1, tie 0.5) against each.

    The newest snapshot is never an opponent (self-play already covers the current policy). Scores are updated per
    game: after ``n`` games with total score ``s`` the EMA keeps ``(1 - alpha) ** n`` of its old value and moves the
    rest to ``s / n``. Unplayed snapshots start at ``prior``.
    """

    def __init__(self, size: int = 12, power: float = 2.0, floor: float = 0.1, alpha: float = 0.02,
                 prior: float = 0.5):
        self.size, self.power, self.floor, self.alpha, self.prior = size, power, floor, alpha, prior
        self.entries: list[dict] = []
        self.next_id = 0

    def __len__(self) -> int:
        return len(self.entries)

    def add(self, state: dict, minutes: float) -> int:
        self.entries.append(dict(id=self.next_id, minutes=round(float(minutes), 2), state=state, win=self.prior,
                                 games=0))
        self.next_id += 1
        self.entries = self.entries[-self.size:]
        return self.next_id - 1

    def candidates(self) -> list[dict]:
        return self.entries[:-1]

    def weights(self) -> np.ndarray:
        return pfsp_weights([e["win"] for e in self.candidates()], self.power, self.floor)

    def sample(self, rng) -> dict | None:
        """A PFSP-weighted opponent snapshot, or None while the pool has fewer than two snapshots."""
        candidates = self.candidates()
        if not candidates:
            return None
        return candidates[int(rng.choice(len(candidates), p=self.weights()))]

    def record(self, snapshot_id, games: float, score: float) -> None:
        """Credit ``games`` finished games with total learner score ``score`` to a snapshot (if still pooled)."""
        if snapshot_id is None or games <= 0:
            return
        for entry in self.entries:
            if entry["id"] == snapshot_id:
                keep = (1.0 - self.alpha) ** games
                entry["win"] = keep * entry["win"] + (1.0 - keep) * score / games
                entry["games"] += int(games)
                return

    def summary(self) -> list:
        """[minutes, learner score EMA, games] per opponent candidate, for the metrics log."""
        return [[e["minutes"], round(e["win"], 3), e["games"]] for e in self.candidates()]

    def state_dict(self) -> dict:
        return {"entries": self.entries, "next_id": self.next_id}

    def load_state_dict(self, state) -> None:
        """Accepts this class's state or the first run's plain list of state dicts."""
        if isinstance(state, list):
            self.entries = [dict(id=i, minutes=0.0, state=s, win=self.prior, games=0) for i, s in enumerate(state)]
            self.next_id = len(state)
        else:
            self.entries, self.next_id = list(state["entries"]), int(state["next_id"])
        self.entries = self.entries[-self.size:]
