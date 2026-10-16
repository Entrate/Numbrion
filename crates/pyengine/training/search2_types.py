"""Shared contract for the two-ply search (scratch/search2/PLAN.md, "Decisions after review").

Every search2 branch builds on these types. Change them only through the orchestrator, so the core, the fused
expansion, the budget controller and the evaluation stay mergeable.

Conventions:
* Action codes are the engine's 0..46 per slot (46 = pass). A side that needs no action at a root uses (-1, -1).
* "Our" side is ``side`` (0 or 1); values are always from our perspective, in [-1, 1] (+1 win, -1 loss, 0 tie).
* Times are ``time.monotonic()`` seconds for deadlines and milliseconds for durations and costs.
* Arrays are numpy; ``[n, ...]`` shapes are given in the field docs.
"""
from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np


# ---- configuration -------------------------------------------------------------------------------------------------

@dataclass
class HalvingRound:
    """One round of sequential halving over our root candidates. Rounds run in order. Each keeps the ``keep``
    best of the survivors by the current two-ply estimate (the first round starts from the shortlist) and deepens
    them with ``worlds`` x ``chance`` samples. A later round reuses an earlier round's samples (worlds and chance
    draws are nested: round r's samples include round r-1's), so it only adds new ones."""
    keep: int
    worlds: int
    chance: int


@dataclass
class Depth2Config:
    """Second-layer search settings (``SearchConfig.depth2``; parsed from a dict by ``from_dict``).

    Fixed sizes. The budget controller (training/search_budget.py) may return a scaled copy per decision."""
    enabled: bool = False
    # Root shortlist: the top ``shortlist`` pairs by layer-1 x, plus the best pair by layer-1 q if missing.
    shortlist: int = 4
    rounds: tuple = (HalvingRound(keep=4, worlds=4, chance=1), HalvingRound(keep=2, worlds=8, chance=2))
    # Opponent replies deepened per (world, our candidate): the top ``theirs_top`` by the layer-1 opponent strategy
    # y_k, plus up to ``danger`` replies with the lowest layer-1 payoff A1[k, i, j] among those with y_k[j] >= danger_min.
    theirs_top: int = 2
    danger: int = 1
    danger_min: float = 0.02
    # Layer 2 at each leaf: our and the opponent's candidates, chance seeds per leaf.
    ours: int = 8
    theirs: int = 6
    seeds: int = 2
    # Forced replacements (faints, pivots) after the root turn are answered by the policy (argmax) and stepped
    # through, at most this many times, so that layer 2 starts at a move turn.
    replacement_steps: int = 2
    # "correction": Q2[i] = Q1[i] + mean_worlds sum_{j deepened} y_k[j] (V2[k,i,j] - A1[k,i,j]) (opponent fixed at
    # layer-1 y; undeepened replies keep their layer-1 values). "resolve": re-solve the reduced game (ablation).
    backup: str = "correction"
    # "shared": our layer-2 strategy is shared by the worlds' leaves that look the same to us (same root pair,
    # same observed opponent reply, same chance index), a Bayesian game per group as at layer 1. "independent":
    # solved per leaf (ablation; optimistic: it fuses strategies across hidden information).
    info: str = "shared"
    # Layer-2 leaf value: "mixed" = x2 . q2, "argmax" = q2[argmax x2].
    leaf_value: str = "mixed"
    # Final pick (and halving score) among the deepened candidates, with Q2 clipped to [-1, 1]:
    # "anchored": argmax of log(prior[i]) + Q2[i] / anchor, the same KL-anchored best response layer 1 makes, on the
    # deeper values (prior = layer 1's our_prior, anchor = SearchConfig.anchor); "q": argmax Q2.
    pick: str = "anchored"
    # Also deepen, per (world, candidate), the likeliest opponent reply that uses a sleep move (y >= danger_min)
    # when none of the chosen replies does: one ply cannot see what sleep costs, so A1 rarely ranks it as a danger.
    threat_reply: bool = True

    @classmethod
    def from_dict(cls, values: dict | None) -> "Depth2Config":
        values = dict(values or {})
        if "rounds" in values:
            values["rounds"] = tuple(r if isinstance(r, HalvingRound) else HalvingRound(**r) for r in values["rounds"])
        known = {k: v for k, v in values.items() if k in cls.__dataclass_fields__}
        return cls(**known)

    def to_dict(self) -> dict:
        out = {k: getattr(self, k) for k in self.__dataclass_fields__}
        out["rounds"] = [dict(keep=r.keep, worlds=r.worlds, chance=r.chance) for r in self.rounds]
        return out


# ---- layer 1 -> layer 2 ----------------------------------------------------------------------------------------------

@dataclass
class Layer1:
    """What ``run_search`` hands to the second layer (raw, before any refusal imputation)."""
    side: int
    worlds: list                    # numbrion.Battle per world (log=True), in layer-1 order
    states: tuple                   # (TrackerState side 0, TrackerState side 1) at the root
    ours: np.ndarray                # [N, 2] our candidate pairs
    our_prior: np.ndarray           # [N] prior used by solve
    their: list                     # per world: [M_k, 2] opponent pairs, or None when the opponent need not act
    their_prior: list               # per world: [M_k] prior, or None
    legal: list                     # per world: [N] bool, our pair legal in that world (raw engine mask)
    payoffs: list                   # per world: [N, M_k] layer-1 payoffs A1 (after refusal imputation, as solved)
    x: np.ndarray                   # [N] layer-1 strategy
    ys: list                        # per world: [M_k] layer-1 opponent strategy
    q: np.ndarray                   # [N] layer-1 values against ys
    seeds: list                     # per world: [S, 4] the layer-1 chance seeds (cells of one world and seed share)
    # Optional hints for the budget controller (plan_depth2): e.g. {"x_top": float, "q_gap": float,
    # "ours_left": int, "foes_left": int, "threats": list[str]} (threats = known dangerous foe moves such as sleep
    # moves, revealed or in most worlds). None: no hints.
    difficulty: dict | None = None


# ---- expansion -----------------------------------------------------------------------------------------------------

@dataclass
class Expansion:
    """Cells stepped from their roots to the next decision boundary, kept for a further search. Index ``c`` is the
    input cell order throughout. Produced by ``search2.expand`` (Python path) or ``SearchGrid.expand`` (Rust path);
    both must agree bit for bit on everything except timing.

    The policy inputs are each side's own observation at the leaf: its root tracker fed with its player-view lines
    from the root to the leaf (fed exactly once), encoded with its leaf request. Policy masks are request-derived
    (``training.protocol.request_mask``) so hidden traps and restrictions are not revealed; ``engine_joint`` is the
    world's true joint mask, used only to model refusals."""
    ended: np.ndarray               # [n] bool
    winner: np.ndarray              # [n] int8, -1 tie or running
    needs_action: np.ndarray        # [n, 2] bool
    request_kind: np.ndarray        # [n, 2] uint8: 0 none, 1 move, 2 replacement
    leaves: list                    # [n] numbrion.Battle, or None for ended cells
    trackers: list                  # [n] (TrackerState side 0, TrackerState side 1) at the leaf, None if ended
    requests: list                  # [n] (request JSON side 0 | None, side 1 | None)
    policy: dict                    # keys ids/floats/field/token_features/action_features/guard, shape [n, 2, ...];
    #                                 rows of a side that needs no action are zero
    request_joint: np.ndarray       # [n, 2, 47, 47] bool, request-derived joint mask per side (False if no action)
    engine_joint: np.ndarray        # [n, 2, 47, 47] bool, the world's engine joint mask per side
    timing: dict = field(default_factory=dict)


# ---- budget --------------------------------------------------------------------------------------------------------

@dataclass
class Stage:
    """Measured cost of one stage, for the cost model: ``units`` of work (cells, policy rows, value rows, roots)
    took ``ms``."""
    name: str                       # "worlds", "root_policy", "opp_policy", "grid1", "value1", "expand",
    #                                 "leaf_policy", "grid2", "value2", "solve2"
    units: int
    ms: float


@dataclass
class Plan:
    """The controller's plan for one decision: layer-1 worlds, and layer 2 (None: skip it). ``chunk_ms`` caps one
    uninterruptible unit of layer-2 work (a grid call or a network batch), so a stage that would overrun the
    deadline is not started."""
    worlds: int
    depth2: Depth2Config | None
    budget_ms: float                # total target for this decision
    deadline: float | None          # monotonic seconds; never exceeded
    chunk_ms: float = 400.0
    started: float | None = None    # monotonic seconds when the decision began (request receipt or decide())
