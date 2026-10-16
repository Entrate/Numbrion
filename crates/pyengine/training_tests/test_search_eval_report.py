"""search_eval reporting: capped games, per-decision times, the second layer's outcomes, config loading, and
``--positions-out`` positions that replay to the same decision point (tools/positions.py)."""
from __future__ import annotations

import gzip
import json
import sys
from pathlib import Path
from types import SimpleNamespace

import numpy as np
import pytest
import torch

numbrion = pytest.importorskip("numbrion")

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import search_eval  # noqa: E402
from tools.positions import derive_seed, new_player, replay_position  # noqa: E402

POOL = HERE / "testdata/pool-400.txt.gz"


def game(score, ms=(), sources=(), depth2=()):
    return dict(score=score, ended=score is not None, rejected=0, stats={}, worlds_count=0, worlds_mean=None,
                opponent_worlds_count=0, opponent_worlds_mean=None,
                decisions={"search": dict(ms=list(ms), source=list(sources), depth2=list(depth2)),
                           "opponent": dict(ms=[1.0], source=["policy"], depth2=[])})


def test_capped_games_are_counted_and_scored_as_ties_on_a_second_line():
    results = [dict(pair=0, games=[game(1.0), game(0.0)]),
               dict(pair=1, games=[game(1.0), game(None)]),
               dict(pair=2, games=[game(None), game(None)])]
    summary = search_eval.summarize(results, 1.0)
    assert (summary["pairs"], summary["score"]) == (1, 0.5)
    assert (summary["capped_games"], summary["capped_pairs"], summary["unfinished"]) == (3, 2, 3)
    assert summary["pairs_all"] == 3
    assert summary["score_capped_as_ties"] == pytest.approx((0.5 + 0.75 + 0.5) / 3)
    lines = search_eval.progress_lines(summary, 3, 3)
    assert "capped: 3 games in 2 pairs" in lines[1] and "as ties: score 0.583" in lines[1]
    no_cap = search_eval.progress_lines(search_eval.summarize(results[:1], 1.0), 1, 1)
    assert not any("capped" in line for line in no_cap)


def test_decision_times_per_side():
    results = [dict(pair=0, games=[game(1.0, ms=[100, 200, 300, 5], sources=["search"] * 3 + ["policy"]),
                                   game(0.0, ms=[400], sources=["search"])])]
    timing = search_eval.summarize(results, 1.0)["timing"]
    searched = timing["search"]["search"]
    assert (searched["count"], searched["mean"], searched["p50"], searched["max"]) == (4, 250.0, 250.0, 400.0)
    assert timing["search"]["all"]["count"] == 5
    assert timing["opponent"]["search"] is None and timing["opponent"]["all"]["count"] == 2
    assert search_eval.time_stats([]) is None


def test_depth2_outcomes():
    info = search_eval.depth2_info
    assert info({"source": "search"}) is None
    # training/search2.py blocks
    assert info({"depth2": {"rounds": 2, "chosen": "move 1, move 2", "changed": True, "skipped": None,
                            "total_ms": 800.0}}) == dict(ran=True, changed=True, reason=None, partial=None, ms=800.0)
    assert info({"depth2": {"rounds": 1, "chosen": "move 1, move 2", "changed": False, "skipped": "deadline",
                            "total_ms": 500.0}}) == dict(ran=True, changed=False, reason=None, partial="deadline",
                                                         ms=500.0)
    assert info({"depth2": {"skipped": "the budget planned no second layer", "rounds": 0, "changed": False}}) == \
        dict(ran=False, changed=False, reason="the budget planned no second layer", partial=None, ms=None)
    assert info({"depth2": {"rounds": 0, "chosen": None, "changed": False, "skipped": "stopped",
                            "timing": {"expand": 10, "grid2": 30}}}) == \
        dict(ran=False, changed=False, reason="stopped", partial=None, ms=40)
    ran = info({"depth2": {"rounds": 2, "chosen": "a", "changed": True, "total_ms": 900}})
    kept = info({"depth2": {"rounds": 1, "chosen": "b", "changed": False, "skipped": "deadline", "total_ms": 700}})
    skipped = info({"depth2": {"rounds": 0, "skipped": "one candidate", "changed": False}})
    results = [dict(pair=0, games=[game(1.0, ms=[1, 2, 3, 4], sources=["search"] * 4,
                                        depth2=[ran, kept, skipped]), game(0.0)])]
    d2 = search_eval.summarize(results, 1.0)["depth2"]
    assert set(d2) == {"search"}
    d = d2["search"]
    assert (d["searched"], d["ran"], d["changed"]) == (4, 2, 1)
    assert d["skipped"] == {"one candidate": 1} and d["stopped_early"] == {"deadline": 1}
    assert (d["ran_share"], d["changed_share"], d["changed_of_ran"]) == (0.5, 0.25, 0.5)
    assert d["layer2_ms"]["mean"] == 800.0
    assert search_eval.summarize([dict(pair=0, games=[game(1.0), game(0.0)])], 1.0)["depth2"] is None


def test_search_configs_from_json_or_files(tmp_path):
    assert search_eval.load_search_arg("off") == (False, {})
    assert search_eval.load_search_arg('{"worlds": 4}') == ({"worlds": 4}, {})
    path = tmp_path / "2ply.json"
    path.write_text(json.dumps({"name": "x", "search": {"worlds": 8, "depth2": {"enabled": True}},
                                "budget": {"target_ms": 3000}}))
    assert search_eval.load_search_arg(str(path)) == ({"worlds": 8, "depth2": {"enabled": True}},
                                                      {"budget": {"target_ms": 3000}})
    bare = tmp_path / "bare.json"
    bare.write_text(json.dumps({"worlds": 2}))
    assert search_eval.load_search_arg(str(bare)) == ({"worlds": 2}, {})


def test_depth2_must_reach_the_bot():
    bot = SimpleNamespace(search=SimpleNamespace(worlds=4))
    with pytest.raises(SystemExit, match="depth2"):
        search_eval.check_depth2({"depth2": {"enabled": True}}, bot, "--search")
    search_eval.check_depth2({"worlds": 4}, bot, "--search")
    search_eval.check_depth2({"depth2": {}}, SimpleNamespace(search=SimpleNamespace(depth2=None)), "--search")


def test_players_get_seeds_when_the_bot_takes_them():
    calls = []
    seeded = SimpleNamespace(new_player=lambda room, seed=None: calls.append((room, seed)) or "p")
    plain = SimpleNamespace(new_player=lambda room: calls.append((room,)) or "q")
    assert new_player(seeded, "a", 5) == ("p", True)
    assert new_player(plain, "b", 5) == ("q", False)
    assert new_player(seeded, "c", None) == ("p", False)
    assert calls == [("a", 5), ("b",), ("c", None)]
    assert derive_seed(7, 1, 0, 0) == derive_seed(7, 1, 0, 0) != derive_seed(7, 1, 0, 1)


@pytest.fixture(scope="module")
def bot(tmp_path_factory):
    if not getattr(numbrion, "SEARCH", False):
        pytest.skip("needs a search wheel")
    from training.dex import load_dex
    from training.model import Model
    from training.search import SearchBot

    torch.manual_seed(0)
    path = tmp_path_factory.mktemp("eval") / "random.pt"
    torch.save({"model": Model(load_dex()).state_dict(), "width": 128, "layers": 3}, path)
    return SearchBot(checkpoint=str(path), search={"worlds": 2, "ours": 4, "theirs": 3, "seeds": 1, "threads": 2},
                     pools=[str(POOL)], device="cpu")


def test_positions_replay_to_the_recorded_decision_point(bot):
    teams = [line.strip() for line in gzip.open(POOL, "rt") if line.strip()]
    factory = lambda side, team, seed=None: search_eval.BasePolicy(bot.net, str(bot.dex.path), side, 0.5,  # noqa: E731
                                                                   np.random.default_rng(3))
    result = search_eval.play_game(bot, factory, teams[0], teams[1], (1, 2, 3, 4), 1, False, max_turns=4,
                                   player_seed=11, opponent_seed=12, positions_rate=1.0,
                                   positions_rng=np.random.default_rng(0))
    assert result["capped"] and result["score"] is None  # stopped at the 4-turn cap
    positions = result["positions"]
    assert {p["role"] for p in positions} == {"search", "opponent"}
    decisions = result["decisions"]
    assert len(decisions["search"]["ms"]) == sum(p["role"] == "search" for p in positions)
    for position in positions:
        json.dumps(position)
        replay, _ = replay_position(position)
        battle = replay.battle
        assert battle.turn == position["turn"] and battle.needs_action(position["side"])
        assert battle.request_kind(position["side"]) == position["request_kind"]
    # With a bot: the replayed player decides at the position.
    position = next(p for p in positions if p["role"] == "search" and p["turn"] > 1)
    replay, player = replay_position(position, bot, player_seed=5)
    decision = player.decide()
    assert decision is not None and decision.analysis["source"] in ("search", "policy")
    replay.battle.choose(position["side"], decision.choice)  # a legal choice for the real battle
