"""Online search allocation. All durations are ms; clocks/deadlines are monotonic seconds.

Each stage fits t(n) = fixed + unit*n by exponentially weighted ridge regression,
with nonnegative coefficients and a weak prior. The fit clips high residuals at
1.25x prediction; raw samples still feed the tail estimate and time feedback.
The two coefficients distinguish small policy calls from large batches. The deadline
bound is max(fitted mean, cold prior) times
max(bound_factor, 1 + deviation_k*EMA(relative absolute error), recent_peak*1.1).
The default bound_factor=3 covers a 3x spike *provided that bound holds*. No online
estimator can guarantee a deadline for an arbitrarily slow uninterruptible call.
The driver fallback remains necessary. stats() counts observed bound violations.

Mean allocation uses expected costs, not the conservative chunk bound. A bounded
integral controller updates scale *= (target/recent_mean)**feedback_gain. The caller
must check next_chunk_ok immediately before EVERY grid call and network batch,
including layer 1, split calls to fit Plan.chunk_ms, and use Plan.deadline. Keep
solve/analysis/send within margin_ms. Observe actual calls, not an aggregate with
several fixed costs charged as one batch.

Integration: plan before root policy/world building; replace cfg.worlds with
plan.worlds; plan_depth2 after layer 1; observe once after the whole decision.
Layer1 has no difficulty field in the shared contract. A caller may attach a dict
as layer1.difficulty: factor, floor, known_threat, remaining, forced. Known threats
raise the floor to 1 even for confident x and retain a danger reply where possible.
This is an allocation hook, not a sleep detector: core must perform its threat
check before calling plan_depth2. Request-derived remaining counts are used when
metadata is absent; no sampled opponent moves enter the difficulty calculation.
"""
from __future__ import annotations

import json
import math
import threading
from collections import OrderedDict
from dataclasses import dataclass, replace

import numpy as np

from .search2_types import Depth2Config, Plan, Stage

# Spark CPU starting costs, fixed ms and ms/unit. Calibrate on the serving device.
# Policy rows include encoding/candidate enumeration; grid units include encoding.
PRIORS = {
    "worlds": (3.0, 0.6), "root_policy": (3.0, 4.0), "opp_policy": (3.0, 0.9),
    "grid1": (3.0, 0.035), "value1": (3.0, 0.040), "expand": (4.0, 0.15),
    "leaf_policy": (4.0, 1.5), "grid2": (3.0, 0.035), "value2": (3.0, 0.040),
    "solve2": (2.0, 0.08),
}
NETWORK_STAGES = {"root_policy", "opp_policy", "leaf_policy", "value1", "value2"}
REFERENCES = {"worlds": 32, "root_policy": 1, "opp_policy": 32, "grid1": 4096,
              "value1": 2048, "expand": 64, "leaf_policy": 64, "grid2": 4096,
              "value2": 2048, "solve2": 16}
# Selected by the greater of architecture scale and measured value/policy cost.
# Users may replace this list with budget.presets. Rounds remain nested.
PRESETS = (
    {"name": "spark", "max_scale": 1.8, "worlds": 32, "chance": 4},
    {"name": "medium", "max_scale": 4.5, "worlds": 16, "chance": 3},
    {"name": "large", "max_scale": 1e12, "worlds": 8, "chance": 2},
)


@dataclass
class _Cost:
    fixed: float
    unit: float
    reference: float
    alpha: float = 0.1
    weight: float = 0.0
    sx: float = 0.0
    sxx: float = 0.0
    sy: float = 0.0
    sxy: float = 0.0
    deviation: float = 0.0
    peak: float = 1.0
    samples: int = 0
    violations: int = 0

    def __post_init__(self):
        self.prior = (self.fixed, self.unit * self.reference)

    def mean(self, units):
        return max(0.0, self.fixed + self.unit * units) if units > 0 else 0.0

    def bound(self, units, factor, k):
        if units <= 0:
            return 0.0
        prior = self.prior[0] + self.prior[1] * units / self.reference
        return max(prior, self.mean(units)) * max(factor, 1 + k * self.deviation, self.peak * 1.1)

    def observe(self, units, ms, factor, k):
        predicted = max(self.mean(units), self.prior[0] + self.prior[1] * units / self.reference, 0.01)
        self.violations += int(ms > self.bound(units, factor, k))
        ratio = ms / predicted
        self.deviation += self.alpha * (abs(ratio - 1) - self.deviation)
        self.peak = max(ratio, self.peak * 0.995)
        # Huber-style winsorisation keeps one scheduler stall in the tail model
        # from turning the fitted intercept into an entire large batch's cost.
        # Persistent changes still move the mean each observation. Raw ms above
        # feeds deviation/peak and the decision-average feedback without clipping.
        fitted_ms = min(ms, predicted * 1.25)
        x = units / self.reference
        decay = 1 - self.alpha
        self.weight = decay * self.weight + self.alpha
        self.sx = decay * self.sx + self.alpha * x
        self.sxx = decay * self.sxx + self.alpha * x * x
        self.sy = decay * self.sy + self.alpha * fitted_ms
        self.sxy = decay * self.sxy + self.alpha * x * fitted_ms
        # Intercept extrapolation from only large batches is ill-conditioned. Shrink it
        # more strongly than the slope until small batches provide evidence.
        ridge, intercept_ridge = 0.001, 0.02
        a, b, c = self.weight + intercept_ridge, self.sx, self.sxx + ridge
        y, z = self.sy + intercept_ridge * self.prior[0], self.sxy + ridge * self.prior[1]
        determinant = max(a * c - b * b, 1e-12)
        fixed, slope = (c * y - b * z) / determinant, (a * z - b * y) / determinant
        # Nonnegative least squares on the two boundary axes when unconstrained fit is negative.
        if fixed < 0:
            fixed, slope = 0.0, max(0.0, z / c)
        if slope < 0:
            fixed, slope = max(0.0, y / a), 0.0
        self.fixed, self.unit = fixed, slope / self.reference
        self.samples += 1


class BudgetController:
    @classmethod
    def from_config(cls, budget: dict | None, width: int, layers: int, cell_ms: float) -> "BudgetController":
        return cls(budget or {}, width, layers, cell_ms)

    def __init__(self, budget, width, layers, cell_ms):
        self.target_ms = self._number(budget, "target_ms", 3000, positive=True)
        self.max_ms = self._number(budget, "max_ms", 5500, positive=True)
        self.margin_ms = self._number(budget, "margin_ms", 150)
        self.chunk_ms = self._number(budget, "chunk_ms", 400, positive=True)
        self.bound_factor = self._number(budget, "bound_factor", 3, positive=True)
        self.deviation_k = self._number(budget, "deviation_k", 4)
        self.feedback_gain = self._number(budget, "feedback_gain", 0.08)
        self.mean_alpha = self._number(budget, "mean_alpha", 0.05, positive=True)
        if self.mean_alpha > 1:
            raise ValueError("budget.mean_alpha must be <= 1")
        self.mode = budget.get("mode", "adaptive")
        if self.mode not in {"adaptive", "fixed"}:
            raise ValueError("budget.mode must be adaptive or fixed")
        self.difficulty_floor = self._number(budget, "difficulty_floor", 0.35, positive=True)
        self.scale = 1.0
        self.recent_ms = None
        self.decisions = 0
        self.arch_scale = max(0.25, (width / 128)**2 * layers / 3)
        self.presets = budget.get("presets", PRESETS)
        if not isinstance(self.presets, (list, tuple)) or not self.presets:
            raise ValueError("budget.presets must be a nonempty list")
        for preset in self.presets:
            if (not isinstance(preset, dict) or not all(k in preset for k in ("name", "max_scale", "worlds", "chance"))
                    or any(not math.isfinite(float(preset[k])) or float(preset[k]) <= 0
                           for k in ("max_scale", "worlds", "chance"))):
                raise ValueError("budget.presets entries need a name and positive max_scale, worlds and chance")
        self.models = {}
        overrides = budget.get("priors", {})
        for stage, (fixed, unit) in PRIORS.items():
            if stage in NETWORK_STAGES:
                unit *= self.arch_scale
            # cell_ms was the old combined grid+value estimate; honor conservative caller priors.
            if stage in {"grid1", "grid2"}:
                unit = max(unit, cell_ms * 0.45)
            if stage in {"value1", "value2"}:
                unit = max(unit, cell_ms * 0.55)
            prior = overrides.get(stage, {})
            fixed = self._number(prior, "fixed_ms", fixed)
            unit = self._number(prior, "unit_ms", unit)
            self.models[stage] = _Cost(fixed, unit, REFERENCES[stage])
        self._starts = OrderedDict()
        self._lock = threading.RLock()

    @staticmethod
    def _number(values, key, default, positive=False):
        value = float(values.get(key, default))
        if not math.isfinite(value) or value < 0 or (positive and value == 0):
            raise ValueError(f"budget.{key} must be {'positive' if positive else 'nonnegative'} and finite")
        return value

    def _cost(self, stage, units):
        return self.models[stage].mean(units)

    def plan(self, cfg, cfg2, budget_ms, deadline, now) -> Plan:
        with self._lock:
            # max_ms is a duration cap from this call, never a replacement for the earlier driver deadline.
            end = now + self.max_ms / 1000
            if deadline is not None:
                end = min(end, deadline)
            target = self.target_ms if budget_ms is None else max(0.0, float(budget_ms))
            allowance = min(target * self.scale, self.max_ms - self.margin_ms,
                            max(0.0, (end - now) * 1000 - self.margin_ms))
            depth2 = cfg2 if cfg2 is not None and cfg2.enabled else None
            if self.mode == "fixed":
                worlds = cfg.worlds
            else:
                worlds = 0
                # Layer 1 gets priority; a caller seeing worlds=0 must use its policy fallback.
                for n in range(1, cfg.max_worlds + 1):
                    cells = n * cfg.ours * cfg.theirs * cfg.seeds
                    cost = sum((self._cost("root_policy", 1), self._cost("worlds", n),
                                self._cost("opp_policy", n), self._cost("grid1", cells),
                                self._cost("value1", cells)))
                    if cost > allowance:
                        break
                    worlds = n
            result = Plan(worlds, depth2, max(0.0, allowance), end, self.chunk_ms)
            self._starts[id(result)] = (result, now)
            while len(self._starts) > 128:
                self._starts.popitem(last=False)
            return result

    def _preset(self):
        measured = max(self.models[s].unit / PRIORS[s][1] for s in NETWORK_STAGES)
        scale = max(self.arch_scale, measured)
        return next((p for p in self.presets if scale <= p["max_scale"]), self.presets[-1])

    @staticmethod
    def _remaining(layer1):
        """Read only our request, whose faint count is public; the foe's hidden bench is irrelevant."""
        if not layer1.worlds:
            return None
        raw = layer1.worlds[0].request_json(layer1.side)
        request = json.loads(raw) if isinstance(raw, str) else raw
        mons = (request or {}).get("side", {}).get("pokemon", [])
        return sum("fnt" not in mon.get("condition", "") for mon in mons) if mons else None

    def _difficulty(self, layer1):
        factors = getattr(layer1, "difficulty", {}) or {}
        x, q = np.asarray(layer1.x), np.asarray(layer1.q)
        gap = float(np.sort(q)[-1] - np.sort(q)[-2]) if len(q) >= 2 else 2.0
        factor = 0.45 if len(x) < 2 else (0.55 if x.max() >= 0.9 and gap >= 0.2 else 1.1)
        remaining = factors.get("remaining")
        if remaining is None:
            remaining = self._remaining(layer1)
        if remaining is not None and remaining <= 3:
            factor *= 1.3
        if factors.get("forced"):
            factor *= 0.5
        floor = max(self.difficulty_floor, float(factors.get("floor", 0)))
        if factors.get("known_threat"):
            floor = max(floor, 1.0)
        return min(1.5, max(floor, factor * float(factors.get("factor", 1))))

    def _sizes(self, cfg, layer1):
        """Conservative new leaves in nested rounds, including the extra best-q root."""
        previous = 0
        leaves, groups = 0, 0
        count = min(len(layer1.ours), cfg.shortlist + 1)
        for index, r in enumerate(cfg.rounds):
            survivors = count if index == 0 else min(count, r.keep)
            samples = min(len(layer1.worlds), r.worlds) * r.chance
            leaves += survivors * (cfg.theirs_top + cfg.danger) * max(0, samples - previous)
            groups += survivors * (cfg.theirs_top + cfg.danger) * r.chance
            previous = samples
        return {"expand": leaves * (1 + cfg.replacement_steps),
                "leaf_policy": leaves * 2 * (1 + cfg.replacement_steps),
                "grid2": leaves * cfg.ours * cfg.theirs * cfg.seeds,
                "value2": leaves * cfg.ours * cfg.theirs * cfg.seeds,
                "solve2": groups}

    def _depth_cost(self, cfg, layer1):
        sizes = self._sizes(cfg, layer1)
        # A stage needs several bounded calls, so charge fixed overhead once per chunk.
        total = 0.0
        for stage, units in sizes.items():
            if not units:
                continue
            model = self.models[stage]
            if model.bound(1, self.bound_factor, self.deviation_k) > self.chunk_ms:
                return math.inf
            multiplier = max(self.bound_factor, 1 + self.deviation_k * model.deviation, model.peak * 1.1)
            # Both the learned line and its cold prior floor must admit the chunk.
            capacity = max(1, min(int((self.chunk_ms / multiplier - fixed) / max(unit, 1e-9))
                                  for fixed, unit in ((model.fixed, model.unit),
                                                      (model.prior[0], model.prior[1] / model.reference))))
            total += model.unit * units + model.fixed * math.ceil(units / capacity)
        return total

    def _ladder(self, cfg, worlds):
        """Largest to smallest: samples/rounds, replies, then leaf widths and seeds."""
        preset = self._preset()
        base = tuple(replace(r, worlds=min(worlds, r.worlds)) for r in cfg.rounds)
        if not base:
            return
        grown = []
        current = base
        while max(r.worlds for r in current) < min(worlds, int(preset["worlds"])):
            current = tuple(replace(r, worlds=min(worlds, int(preset["worlds"]), r.worlds * 2)) for r in current)
            grown.append(replace(cfg, rounds=current))
        while max(r.chance for r in current) < int(preset["chance"]):
            current = tuple(replace(r, chance=min(int(preset["chance"]), r.chance * 2)) for r in current)
            grown.append(replace(cfg, rounds=current))
        yield from reversed(grown)
        current_cfg = replace(cfg, rounds=base)
        yield current_cfg
        while any(r.worlds > 1 or r.chance > 1 for r in current_cfg.rounds):
            current_cfg = replace(current_cfg, rounds=tuple(replace(r, worlds=max(1, r.worlds // 2),
                                                                   chance=max(1, r.chance // 2))
                                                           for r in current_cfg.rounds))
            yield current_cfg
        if len(current_cfg.rounds) > 1:
            current_cfg = replace(current_cfg, rounds=current_cfg.rounds[:1])
            yield current_cfg
        # Keep the known-threat danger reply until top replies have been reduced.
        while current_cfg.theirs_top > 1:
            current_cfg = replace(current_cfg, theirs_top=current_cfg.theirs_top - 1)
            yield current_cfg
        while current_cfg.danger > 0:
            current_cfg = replace(current_cfg, danger=current_cfg.danger - 1)
            yield current_cfg
        while current_cfg.ours > 2 or current_cfg.theirs > 2 or current_cfg.seeds > 1:
            current_cfg = replace(current_cfg, ours=max(2, current_cfg.ours // 2),
                                  theirs=max(2, current_cfg.theirs // 2), seeds=max(1, current_cfg.seeds // 2))
            yield current_cfg

    def plan_depth2(self, plan, layer1, now) -> Depth2Config | None:
        with self._lock:
            if plan.depth2 is None or not layer1.worlds or len(layer1.ours) < 2:
                return None
            left = math.inf if plan.deadline is None else (plan.deadline - now) * 1000 - self.margin_ms
            if left <= 0:
                return None
            if self.mode == "fixed":
                return plan.depth2
            entry = self._starts.get(id(plan))
            if entry is None or entry[0] is not plan:
                # An externally constructed Plan lacks a start clock; do not invent remaining target time.
                return None
            elapsed = max(0.0, (now - entry[1]) * 1000)
            available = min(left, plan.budget_ms * self._difficulty(layer1) - elapsed)
            for candidate in self._ladder(plan.depth2, len(layer1.worlds)):
                if self._depth_cost(candidate, layer1) <= available:
                    return candidate
            return None

    def next_chunk_ok(self, stage: str, units: int, now: float, deadline: float | None) -> bool:
        with self._lock:
            if stage not in self.models:
                raise ValueError(f"unknown search stage: {stage}")
            if units < 0:
                raise ValueError("chunk units must be nonnegative")
            bound = self.models[stage].bound(units, self.bound_factor, self.deviation_k)
            return bound <= self.chunk_ms and (deadline is None or
                    now + (bound + self.margin_ms) / 1000 <= deadline)

    def observe(self, stages: list[Stage], total_ms: float) -> None:
        with self._lock:
            if not math.isfinite(total_ms) or total_ms < 0:
                raise ValueError("total_ms must be nonnegative and finite")
            for s in stages:
                if s.name not in self.models:
                    raise ValueError(f"unknown search stage: {s.name}")
                if s.units < 0 or not math.isfinite(s.ms) or s.ms < 0:
                    raise ValueError("stage units and ms must be nonnegative and finite")
                if s.units:
                    self.models[s.name].observe(s.units, s.ms, self.bound_factor, self.deviation_k)
            self.recent_ms = total_ms if self.recent_ms is None else (
                self.recent_ms + self.mean_alpha * (total_ms - self.recent_ms))
            self.decisions += 1
            if self.mode == "adaptive":
                ratio = self.target_ms / max(1, self.recent_ms)
                self.scale = min(2.0, max(0.5, self.scale * min(1.25, max(0.8, ratio))**self.feedback_gain))

    def stats(self) -> dict:
        with self._lock:
            return {"decisions": self.decisions, "recent_ms": self.recent_ms, "scale": self.scale,
                    "preset": self._preset()["name"], "mode": self.mode,
                    "stages": {name: {"fixed_ms": m.fixed, "unit_ms": m.unit, "samples": m.samples,
                                      "relative_deviation": m.deviation, "peak_ratio": m.peak,
                                      "bound_violations": m.violations}
                               for name, m in self.models.items()}}
