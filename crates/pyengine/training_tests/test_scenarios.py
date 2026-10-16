"""The scenario suite (tools/scenarios): every script replays to its target position, broken scripts fail loudly,
choice tokens and expectations, and the runner end to end with a small 1-ply config (a random-weight model, so this
checks mechanics, not strength)."""
from __future__ import annotations

import gzip
import json
import sys
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace

import pytest
import torch

numbrion = pytest.importorskip("numbrion")

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

from tools.scenarios import run as runner  # noqa: E402
from tools.scenarios.core import (ChoiceView, Expect, ScenarioError, Target, build, check_target,  # noqa: E402
                                  slot_token)
from tools.scenarios.library import SCENARIOS  # noqa: E402

POOL = HERE / "testdata/pool-400.txt.gz"
FAMILIES = {"sleep", "yawn", "endgame", "control"}


@pytest.mark.parametrize("scenario", SCENARIOS, ids=lambda s: s.name)
def test_scenario_reaches_its_target(scenario):
    replay = build(scenario)
    assert check_target(scenario, replay) == []
    assert replay.battle.needs_action(scenario.side)


def test_suite_shape():
    names = [s.name for s in SCENARIOS]
    assert len(names) == len(set(names)) >= 16
    assert {s.family for s in SCENARIOS} == FAMILIES
    for s in SCENARIOS:
        assert s.rationale, s.name
        if s.expect is not None:
            assert set(s.expect.accept) | set(s.expect.reject) <= {0, 1}, s.name
    assert sum(s.expect is not None for s in SCENARIOS) >= 10


def test_an_illegal_scripted_choice_fails_loudly():
    scenario = SCENARIOS[0]
    broken = replace(scenario, script=[("move 1 +1, move 1 +1", "move 1 +1, move 4 +2")])  # Quiver Dance takes no target
    with pytest.raises(ScenarioError, match="rejected"):
        build(broken)
    missing = replace(scenario, script=[("move 4, move 1 +1", None), ("move 4, move 1 +1", None)])
    with pytest.raises(ScenarioError, match="boundary 0 is not finished: side.s. .1."):
        build(missing)
    with pytest.raises(ScenarioError, match="already chose"):  # a side twice at one decision point
        _replay_steps(scenario, [dict(side=0, choice="move 4, move 1 +1"), dict(side=0, choice="move 4, move 1 +1")])
    two_v_one = next(s for s in SCENARIOS if s.name == "endgame-2v1-attack-both")
    with pytest.raises(ScenarioError, match="no decision"):  # only p1 replaces its fainted Pikachu
        build(replace(two_v_one, script=[two_v_one.script[0], ("switch 3", "default")]))


def _replay_steps(scenario, steps):
    from tools.positions import Replay, ReplayError

    try:
        Replay(scenario.teams, scenario.seed).run(steps)
    except ReplayError as error:
        raise ScenarioError(str(error)) from None


def test_a_wrong_target_fails_loudly():
    scenario = SCENARIOS[0]
    with pytest.raises(ScenarioError, match="turn 2, expected 3"):
        build(replace(scenario, target=replace(scenario.target, turn=3)))
    with pytest.raises(ScenarioError, match="expected slp"):
        build(replace(scenario, target=Target(mons={(0, "volcarona"): "slp"})))
    with pytest.raises(ScenarioError, match="log lacks"):
        build(replace(scenario, target=Target(log=["|move|p2a: Amoonguss|Rage Powder"])))


def test_slot_tokens_and_expectations():
    request = {"side": {"pokemon": [{"details": "Volcarona, L78"}, {"details": "Talonflame, L82, F"},
                                    {"details": "Great Tusk, L81"}]},
               "active": [{"moves": [{"id": "quiverdance"}, {"id": "fierydance"}]},
                          {"moves": [{"id": "bravebird"}, {"id": "protect"}]}]}
    assert slot_token(request, 0, 0 * 10 + 2 * 2) == "move:quiverdance"
    assert slot_token(request, 0, 1 * 10 + 3 * 2) == "move:fierydance@+1"
    assert slot_token(request, 1, 0 * 10 + 4 * 2 + 1) == "move:bravebird@+2+tera"
    assert slot_token(request, 1, 0 * 10 + 1 * 2) == "move:bravebird@-1"
    assert slot_token(request, 1, 42) == "switch:greattusk"
    assert slot_token(request, 1, 46) == "pass"
    view = ChoiceView("move 2 +1, move 2", ("move:fierydance@+1", "move:protect"))
    assert Expect(reject={0: ["move:quiverdance*"]}).check(view)
    assert not Expect(accept={1: ["move:bravebird*"]}).check(view)
    assert Expect(any_slot=["move:*@+1*"]).check(view)
    assert not Expect(predicate=lambda v: "switch" in v.text).check(view)


@pytest.fixture(scope="module")
def setup(tmp_path_factory):
    """A random-weight checkpoint, and a set pool with the test pool plus the scenarios' opponent teams (so the world
    builder can reproduce the opponents' revealed sets)."""
    from training.dex import load_dex
    from training.model import Model

    root = tmp_path_factory.mktemp("scenarios")
    torch.manual_seed(0)
    checkpoint = root / "random.pt"
    torch.save({"model": Model(load_dex()).state_dict(), "width": 128, "layers": 3}, checkpoint)
    pool = root / "pool.txt.gz"
    with gzip.open(POOL, "rt") as f:
        lines = [line for line in f if line.strip()]
    lines += [s.teams[1 - s.side] + "\n" for s in SCENARIOS]
    with gzip.open(pool, "wt") as f:
        f.writelines(lines)
    config = root / "1ply-small.json"
    config.write_text(json.dumps({"name": "1ply-small", "search": {"worlds": 3, "ours": 4, "theirs": 3, "seeds": 1,
                                                                    "threads": 2}, "torch_threads": 1}))
    return SimpleNamespace(root=root, checkpoint=checkpoint, pool=pool, config=config)


@pytest.mark.skipif(not getattr(numbrion, "SEARCH", False), reason="needs a search wheel")
def test_runner_with_a_1ply_config(setup, monkeypatch, capsys):
    out = setup.root / "report.json"
    monkeypatch.setattr(sys, "argv", ["run.py", "--config", str(setup.config), "--checkpoint", str(setup.checkpoint),
                                      "--set-pool", str(setup.pool), "--repeats", "2", "--out", str(out)])
    runner.main()
    report = json.loads(out.read_text())
    assert [e["name"] for e in report["scenarios"]] == [s.name for s in SCENARIOS]
    sources = set()
    for entry, scenario in zip(report["scenarios"], SCENARIOS):
        result = entry["configs"]["1ply-small"]
        runs = result["runs"]
        assert len(runs) == 2 and all(r["choice"] for r in runs), entry["name"]
        assert all(len(r["tokens"]) == 2 for r in runs)
        sources |= {r["source"] for r in runs}
        if scenario.expect is None:
            assert result["summary"]["pass_rate"] is None
        else:
            assert 0.0 <= result["summary"]["pass_rate"] <= 1.0
        assert sum(result["summary"]["choices"].values()) == 2
    assert "search" in sources
    assert report["totals"]["1ply-small"]["scenarios"] == sum(s.expect is not None for s in SCENARIOS)
    assert "1ply-small" in capsys.readouterr().out


@pytest.mark.skipif(not getattr(numbrion, "SEARCH", False), reason="needs a search wheel")
def test_runner_seeds_make_decisions_repeatable(setup):
    config = json.loads(setup.config.read_text())
    bot = runner.make_bot(config, str(setup.checkpoint), str(setup.pool))
    scenario = next(s for s in SCENARIOS if s.name == "endgame-tera-only-out")
    first = runner.decide_once(scenario, bot, 1234, config)
    second = runner.decide_once(scenario, bot, 1234, config)
    assert first["source"] == "search"
    assert (first["choice"], first["value"]) == (second["choice"], second["value"])


def test_a_depth2_config_needs_the_core_change(setup):
    from training.search import SearchConfig

    if "depth2" in SearchConfig.__dataclass_fields__:
        pytest.skip("SearchConfig has depth2 (search2 core merged)")
    config = dict(json.loads(setup.config.read_text()), search={"worlds": 2, "depth2": {"enabled": True}})
    with pytest.raises(SystemExit, match="depth2"):
        runner.make_bot(config, str(setup.checkpoint), str(setup.pool))
