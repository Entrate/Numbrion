"""Policy move guards for stale first-turn moves and repeated Protect-like moves.

Fake Out and First Impression always fail after the user's first move action since switching in; a Protect-like
move right after a successful one works one time in three. The guards block every target and Tera code of those
moves, but keep a row's mask unchanged when no legal move code would remain, even if switching is possible.
Target-dependent failures (immunities, Prankster into Dark, priority into Psychic Terrain or Armor Tail) are not
blocked: a foe can switch or the partner can change what works before the move executes.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np

STALE, REPEAT_PROTECT = 0, 1  # last axis of Buffers.guard
FIRST_TURN_ONLY = ("fakeout", "firstimpression")


@dataclass(frozen=True)
class MoveGuards:
    stale: bool = True
    repeat_protect: bool = True

    def blocked(self, guard: np.ndarray, slot: int) -> np.ndarray:
        """[rows, 47] bool: codes to block for own active ``slot`` (0 or 1), from ``guard`` [rows, 2, 4, 2].
        Move codes 0-39 map to move slot ``code // 10`` (all targets, with and without Tera); switches and PASS are
        never blocked. Kinds that are switched off contribute nothing."""
        moves = (guard[:, slot, :, STALE] & self.stale) | (guard[:, slot, :, REPEAT_PROTECT] & self.repeat_protect)
        blocked = np.zeros((guard.shape[0], 47), dtype=np.bool_)
        blocked[:, :40] = np.repeat(moves, 10, axis=1)
        return blocked

    def apply(self, mask: np.ndarray, guard: np.ndarray, slot: int) -> tuple[np.ndarray, np.ndarray]:
        """(guarded mask, guarded rows). ``mask`` [rows, 47] bool is not modified; the result is a new array.
        A row keeps its mask when blocking would leave no legal move code (0-39): the move is all the Pokemon can
        use (Choice lock, Encore, last move with PP). ``guarded rows`` [rows] bool: the guard removed at least one
        legal code from that row. PASS-only rows are untouched."""
        blocked = self.blocked(guard, slot)
        blocked &= (mask[:, :40] & ~blocked[:, :40]).any(-1)[:, None]
        return mask & ~blocked, (mask & blocked).any(-1)
