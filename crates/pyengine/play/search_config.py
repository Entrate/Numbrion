"""Shared ladder/advisor validation for opt-in search budgeting.

The whole bot spec already reaches load_bot as kwargs. Keep budget at that level,
with depth2 nested in search; never turn budgeting on for an existing config.
"""
from __future__ import annotations


def validate_search_options(spec: dict) -> None:
    budget = spec.get("budget")
    if budget is not None:
        if not isinstance(budget, dict):
            raise ValueError("bot.budget must be an object")
        from training.search_budget import BudgetController
        BudgetController.from_config(budget, width=128, layers=3, cell_ms=0.3)
    search = spec.get("search")
    if not isinstance(search, dict) or "depth2" not in search:
        return
    depth2 = search["depth2"]
    if depth2 is False or depth2 == "off":
        search["depth2"] = {"enabled": False}
        return
    if not isinstance(depth2, dict):
        raise ValueError("bot.search.depth2 must be an object, false or 'off'")
    from training.search2_types import Depth2Config
    cfg = Depth2Config.from_dict(depth2)
    for name in ("shortlist", "theirs_top", "ours", "theirs", "seeds"):
        value = getattr(cfg, name)
        if isinstance(value, bool) or not isinstance(value, int) or value < 1:
            raise ValueError(f"bot.search.depth2.{name} must be a positive integer")
    for name in ("danger", "replacement_steps"):
        value = getattr(cfg, name)
        if isinstance(value, bool) or not isinstance(value, int) or value < 0:
            raise ValueError(f"bot.search.depth2.{name} must be a nonnegative integer")
    previous = None
    if cfg.enabled and not cfg.rounds:
        raise ValueError("bot.search.depth2.rounds must not be empty when enabled")
    for r in cfg.rounds:
        if any(isinstance(v, bool) or not isinstance(v, int) or v < 1 for v in (r.keep, r.worlds, r.chance)):
            raise ValueError("bot.search.depth2.rounds sizes must be positive integers")
        if previous and (r.worlds < previous.worlds or r.chance < previous.chance or r.keep > previous.keep):
            raise ValueError("bot.search.depth2.rounds must use nested samples and decreasing keep")
        previous = r
    if cfg.backup not in {"correction", "resolve"} or cfg.info not in {"shared", "independent"}:
        raise ValueError("invalid bot.search.depth2 backup or info")
    if cfg.leaf_value not in {"mixed", "argmax"}:
        raise ValueError("invalid bot.search.depth2.leaf_value")
    if not isinstance(cfg.danger_min, (int, float)) or not 0 <= cfg.danger_min <= 1:
        raise ValueError("bot.search.depth2.danger_min must be in [0, 1]")
