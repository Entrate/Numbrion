"""Behaviour diagnostics over the trained decisions of a rollout buffer (logged every update by the trainer).

The first run drifted into an all-out attack style (Tera on turn 1, almost no Protect, Fake Out or status moves);
these shares make that kind of drift visible while training instead of only in the evaluation afterwards.
"""
from __future__ import annotations

import numpy as np

from .dex import Dex
from .encoder import I_MOVE0

N_ACTIONS = 47
PASS = 46


class ActionUsage:
    """Shares of action kinds among the learner's slot decisions on move turns.

    A slot decision counts when the slot could use a move (any move code legal), so forced switches after a KO and
    PASS are excluded. A move code's move is the own active's move in that slot (tokens 0/1, ids ``I_MOVE0..+3``),
    the same mapping the model scores actions with. ``choice_slots`` is the mean number of slots per trained row
    with two or more legal codes; it shows how much forced decisions dilute the logged entropy.
    """

    def __init__(self, dex: Dex):
        self.protect = np.asarray(dex.move_stalling, dtype=bool).copy()
        self.status = (np.asarray(dex.move_category) == 0) & ~self.protect
        self.status[0] = False
        self.fake_out = np.zeros(dex.n_moves, dtype=bool)
        if "fakeout" in dex.move_index:
            self.fake_out[dex.move_index["fakeout"]] = True

    def __call__(self, data: dict) -> dict:
        train = data["train"].reshape(-1)
        if not train.any():
            return {}
        codes = np.stack((data["a0"].reshape(-1)[train], data["a1"].reshape(-1)[train]), 1)  # [N, 2]
        masks = np.stack((data["mask0"].reshape(-1, N_ACTIONS)[train], data["mask1"].reshape(-1, N_ACTIONS)[train]), 1)
        steps, rows = data["a0"].shape
        moves = data["ids"][:, :, 0:2, I_MOVE0:I_MOVE0 + 4].reshape(steps * rows, 2, 4)[train]  # [N, 2, 4]
        decision = masks[..., :40].any(-1)  # [N, 2]
        is_move = codes < 40
        move_id = np.take_along_axis(moves, np.where(is_move, codes // 10, 0)[..., None], -1)[..., 0]
        move_id = np.where(is_move, move_id, 0)
        tera = is_move & (codes % 2 == 1)
        tera_legal = decision & masks[..., 1:40:2].any(-1)
        kinds = dict(tera=tera, switch=(codes >= 40) & (codes < PASS), protect=self.protect[move_id],
                     status=self.status[move_id], fake_out=self.fake_out[move_id])
        n = max(int(decision.sum()), 1)
        out = {f"use_{k}": round(float((v & decision).sum()) / n, 4) for k, v in kinds.items()}
        out["use_tera_when_legal"] = round(float(tera[tera_legal].mean()), 4) if tera_legal.any() else None
        out["use_decisions"] = int(decision.sum())
        out["choice_slots"] = round(float((masks.sum(-1) > 1).sum(-1).mean()), 3)
        return out
