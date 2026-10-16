"""A minimal stand-in for ``training.search_budget.BudgetController`` (Sol-2's module), used by ``SearchBot`` only
while that module is missing. Same interface (docs/design/SEARCH2-PLAN.md, "Budget controller interface"), none of
the planning: layer 1 gets the worlds that fit ``budget_ms`` at the measured cost per cell (as
``SearchBot.config_for``), layer 2 runs its configured sizes, and a chunk is refused when a conservative bound on its
cost passes the deadline. Delete it once search_budget.py lands."""
from __future__ import annotations

from dataclasses import replace

from .search2_types import Depth2Config, Plan, Stage

UNIT_MS = {"expand": 0.1, "leaf_policy": 0.6, "grid2": 0.3, "value2": 0.1, "solve2": 0.5}
SAFETY, FIXED_MS, MARGIN_MS = 2.0, 15.0, 120.0


class BudgetController:
    def __init__(self, budget: dict, cell_ms: float):
        self.config = dict(budget or {})
        self.cell_ms = cell_ms
        self.fixed_ms = 60.0
        self.unit_ms = dict(UNIT_MS)
        self.decisions = 0
        self.total_ms = 0.0

    @classmethod
    def from_config(cls, budget: dict | None, width: int, layers: int, cell_ms: float) -> "BudgetController":
        del width, layers
        return cls(budget or {}, cell_ms)

    def plan(self, cfg, cfg2: Depth2Config | None, budget_ms: float | None, deadline: float | None,
             now: float) -> Plan:
        del now
        worlds = cfg.worlds
        if budget_ms:
            per_world = cfg.ours * cfg.theirs * cfg.seeds * self.cell_ms
            worlds = max(2, min(cfg.max_worlds, int((budget_ms - self.fixed_ms) // max(per_world, 1e-3))))
        return Plan(worlds=worlds, depth2=cfg2 if cfg2 is not None and cfg2.enabled else None,
                    budget_ms=float(budget_ms or 0.0), deadline=deadline)

    def plan_depth2(self, plan: Plan, layer1, now: float) -> Depth2Config | None:
        del layer1, now
        return None if plan.depth2 is None else replace(plan.depth2)

    def next_chunk_ok(self, stage: str, units: int, now: float, deadline: float | None) -> bool:
        if deadline is None:
            return True
        predicted = SAFETY * units * self.unit_ms.get(stage, 0.5) + FIXED_MS
        return now + (predicted + MARGIN_MS) / 1000.0 <= deadline

    def observe(self, stages: list[Stage], total_ms: float) -> None:
        self.decisions += 1
        self.total_ms += total_ms
        cells = sum(s.units for s in stages if s.name == "grid1")
        cell_part = sum(s.ms for s in stages if s.name in ("opp_policy", "grid1", "value1"))
        if cells:
            self.cell_ms += 0.2 * (cell_part / cells - self.cell_ms)
        for s in stages:
            if s.name in self.unit_ms and s.units > 0:
                self.unit_ms[s.name] += 0.2 * (s.ms / s.units - self.unit_ms[s.name])

    def stats(self) -> dict:
        return dict(stub=True, decisions=self.decisions, mean_ms=self.total_ms / max(self.decisions, 1),
                    cell_ms=round(self.cell_ms, 4), unit_ms={k: round(v, 4) for k, v in self.unit_ms.items()})
