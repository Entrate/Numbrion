"""Opt-in search2 settings reach both serving entry points and old/new constructors."""
import asyncio
import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from play.driver import BotWorker
from play.ladder import ConfigError, load_config
from play.search_config import validate_search_options
from play.server import load_config as server_config
from training.search_budget import BudgetController


def test_ladder_example_and_existing_configs():
    here = Path(__file__).parent
    for name in ("ladder.json", "ladder-mac.json"):
        assert "budget" not in load_config(here / name)["bot"]
    spec = load_config(here / "ladder-search2.json")["bot"]
    assert spec["budget"]["target_ms"] == 3000
    assert spec["search"]["depth2"]["rounds"][1]["worlds"] == 8


def test_advisor_preserves_budget_and_accepts_depth2_off(tmp_path):
    spec = {"name": "Example", "factory": "training.search:SearchBot", "budget": {"mode": "fixed"},
            "search": {"depth2": "off"}}
    path = tmp_path / "bots.json"
    path.write_text(json.dumps({"bots": [spec]}))
    specs, advisor = server_config(path)
    assert specs[0]["budget"] == spec["budget"]
    assert specs[0]["search"]["depth2"] == {"enabled": False}
    assert advisor == "Example"


@pytest.mark.parametrize("extra", [{"budget": []}, {"budget": {"mode": "wrong"}},
    {"search": {"depth2": {"enabled": True, "rounds": []}}},
    {"search": {"depth2": {"ours": 0}}},
    {"search": {"depth2": {"rounds": [{"keep": 2, "worlds": 4, "chance": 2},
                                         {"keep": 3, "worlds": 2, "chance": 1}]}}}])
def test_invalid_search_options(extra):
    with pytest.raises((ValueError, TypeError)):
        validate_search_options(extra)
    with pytest.raises(ConfigError):
        load_config(None, base={"account": "Example", "bot": {"factory": "x:y", **extra}})


@pytest.mark.parametrize("constructor", ["old", "new", "unconfigured"])
def test_worker_passes_budget_and_compatibility_controller(monkeypatch, constructor):
    spec = {"name": "Example", "factory": "training.search:SearchBot"}
    if constructor != "unconfigured":
        spec["budget"] = {"target_ms": 2400}
    bot = SimpleNamespace(net=SimpleNamespace(width=192, layers=4), cell_ms=.1)
    expected = BudgetController.from_config(spec.get("budget"), 192, 4, .1) if constructor == "new" else None
    if expected:
        bot.budget = expected
    seen = []
    def factory(passed):
        seen.append(passed)
        return bot
    monkeypatch.setattr("training.player.load_bot", factory)
    worker = BotWorker(spec)
    try:
        asyncio.run(worker.load())
        assert worker.failed is None and worker.ready.is_set()
        assert seen == [spec]
        if constructor == "unconfigured":
            assert not hasattr(bot, "budget")
        else:
            assert bot.budget.target_ms == 2400
            if expected:
                assert bot.budget is expected
    finally:
        worker.executor.shutdown()
