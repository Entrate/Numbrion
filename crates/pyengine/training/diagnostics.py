"""Behaviour diagnostics over the trained decisions of a rollout buffer (logged every update by the trainer).

The first run drifted into an all-out attack style (Tera on turn 1, almost no Protect, Fake Out or status moves);
these shares make that kind of drift visible while training instead of only in the evaluation afterwards.
"""
from __future__ import annotations

import numpy as np

from .damage import ACTION_COLUMNS
from .dex import Dex
from .encoder import I_MOVE0, LEGACY_ACTION_FEATURES, PF

FAIL, FLINCH_A, FLINCH_B, PROTECT = (LEGACY_ACTION_FEATURES + ACTION_COLUMNS.index(k)
                                     for k in ("fail", "flinch_a", "flinch_b", "protect"))

N_ACTIONS = 47
PASS = 46


class ActionUsage:
    """Shares of action kinds among the learner's slot decisions on move turns.

    A slot decision counts when the slot could use a move (any move code legal), so forced switches after a KO and
    PASS are excluded. A move code's move is the own active's move in that slot (tokens 0/1, ids ``I_MOVE0..+3``),
    the same mapping the model scores actions with. ``choice_slots`` is the mean number of slots per trained row
    with two or more legal codes; it shows how much forced decisions dilute the logged entropy.

    From the action features: ``fake_out_ready`` (share of slot decisions with a legal Fake Out that would flinch a
    foe), ``use_fake_out_when_ready`` (how often one is then used), ``fake_out_stale`` (share of Fake Outs by a
    Pokemon that already used a move action since switching in, so they fail) and ``protect_repeat`` (share of
    Protects right after a successful one, which work one time in three). With the move guards (training.guards)
    on, these two only count what the guards let through: the only-move exception, and Pokemon the tracker can't
    place (our disguised Zoroark), whose ``fresh`` reads 0. They should sit near zero.
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
        if "action_features" in data:
            features = data["action_features"].reshape(steps * rows, 2, 40, -1)[train]  # [N, 2, 40, F]
            works = (features[..., FAIL] < 0.5) & (features[..., FLINCH_A] + features[..., FLINCH_B] > 0)
            code_moves = moves[:, :, np.arange(40) // 10]  # [N, 2, 40]
            ready = decision & (self.fake_out[code_moves] & masks[..., :40] & works).any(-1)
            chosen = np.where(is_move, codes, 0)[..., None]
            stale = data["floats"][:, :, 0:2, PF["fresh"]].reshape(steps * rows, 2)[train] < 0.5
            repeat = np.take_along_axis(features[..., PROTECT], chosen, -1)[..., 0] < 0.99
            fake_out, protects = kinds["fake_out"] & decision, kinds["protect"] & decision
            share = lambda part, whole: round(float(part.sum()) / whole.sum(), 4) if whole.any() else None  # noqa: E731
            out["fake_out_ready"] = round(float(ready.sum()) / n, 4)
            out["use_fake_out_when_ready"] = share(fake_out & ready, ready)
            out["fake_out_stale"] = share(fake_out & stale, fake_out)
            out["protect_repeat"] = share(protects & repeat, protects)
        out["choice_slots"] = round(float((masks.sum(-1) > 1).sum(-1).mean()), 3)
        return out
