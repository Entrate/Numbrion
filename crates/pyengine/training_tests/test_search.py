"""training.search: the matrix-game solver, the opponent-view HP rule, and whole searched battles driven only by
player-view lines and requests (a random-weight model, so this checks mechanics, not strength)."""
from __future__ import annotations

import gzip
import json
from pathlib import Path

import numpy as np
import pytest
import torch

numbrion = pytest.importorskip("numbrion")
if not (numbrion.SEARCH and hasattr(numbrion, "SearchGrid")):
    pytest.skip("needs a search wheel with the fused grid", allow_module_level=True)

from training.dex import load_dex  # noqa: E402
from training.model import Model  # noqa: E402
from training.search import SearchBot, describe_code, public_hp, solve  # noqa: E402

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "crates/pyengine/testdata/pool-400.txt.gz"


def test_best_response_to_the_opponent_prior():
    payoff = np.array([[0.5, -0.5], [0.1, 0.1], [-0.2, 0.9]])
    prior = np.array([0.8, 0.2])
    x, ys, q = solve([payoff], [np.ones(3, bool)], np.ones(1), np.full(3, 1 / 3), [prior], anchor=1e-3,
                     opponent_anchor=0.0, iterations=10)
    assert np.allclose(q, payoff @ prior)
    assert int(np.argmax(x)) == 0 and np.allclose(ys[0], prior)


def test_regularised_equilibrium_mixes_matching_pennies():
    payoff = np.array([[1.0, -1.0], [-1.0, 1.0]])
    x, ys, q = solve([payoff], [np.ones(2, bool)], np.ones(1), np.full(2, 0.5), [np.array([0.9, 0.1])],
                     anchor=0.05, opponent_anchor=0.05, iterations=2000)
    assert abs(x[0] - 0.5) < 0.1 and abs(ys[0][0] - 0.5) < 0.1


def test_pairs_illegal_in_a_world_are_scored_on_the_other_worlds():
    a = np.array([[1.0], [0.0]])
    b = np.array([[-1.0], [0.5]])
    _, _, q = solve([a, b], [np.array([True, True]), np.array([False, True])], np.ones(2), np.full(2, 0.5),
                    [np.ones(1), np.ones(1)], anchor=0.1, opponent_anchor=0.0, iterations=1)
    assert np.allclose(q, [1.0, 0.25])


def test_public_hp_hides_only_our_exact_hp():
    lines = ["|switch|p1a: Muk|Muk, L82|259/260 par", "|-damage|p2a: Ditto|50/100", "|-heal|p1b: Mew|0 fnt",
             "|-damage|p1b: Mew|1/300|[from] psn", "|move|p1a: Muk|Tackle|p2a: Ditto"]
    assert public_hp(lines, 0) == ["|switch|p1a: Muk|Muk, L82|99/100 par", lines[1], lines[2],
                                   "|-damage|p1b: Mew|1/100|[from] psn", lines[4]]
    assert public_hp(lines, 1) == lines


def test_describe_code_targets():
    request = {"side": {"pokemon": [{"details": "Amoonguss, L86"}, {"details": "Infernape, L82"},
                                    {"details": "Toucannon, L88"}]},
               "active": [{"moves": [{"move": "Spore"}]}, {"moves": [{"move": "Knock Off"}]}]}
    assert describe_code(request, 0, 0 * 10 + 3 * 2, ("Toxicroak", "Illumise")) == "Spore -> Toxicroak"
    assert describe_code(request, 1, 0 * 10 + 4 * 2 + 1, ("Toxicroak", "Illumise")) == "Knock Off -> Illumise + Tera"
    assert describe_code(request, 0, 0 * 10 + 0 * 2, ("", "")) == "Spore -> ally"  # code target 0 = own slot b
    assert describe_code(request, 1, 0 * 10 + 0 * 2, ("", "")) == "Knock Off -> self"
    assert describe_code(request, 0, 42, ("", "")) == "switch to Toucannon"


@pytest.fixture(scope="module")
def bot(tmp_path_factory):
    torch.manual_seed(0)
    path = tmp_path_factory.mktemp("search") / "random.pt"
    torch.save({"model": Model(load_dex()).state_dict(), "width": 128, "layers": 3}, path)
    return SearchBot(checkpoint=str(path), search={"worlds": 2, "ours": 4, "theirs": 3, "seeds": 1, "threads": 2,
                                                   "opponent_anchor": 0.1},
                     pools=[str(POOL)], device="cpu")


def test_searched_battles_use_only_the_player_view(bot):
    teams = [line.strip() for line in gzip.open(POOL, "rt") if line.strip()]
    sources = {"search": 0, "policy": 0}
    for game in range(3):
        battle = numbrion.Battle((game, 2, 3, 4), p1=teams[2 * game], p2=teams[2 * game + 1], log=True)
        battle.start()
        player = bot.new_player(f"test-{game}")  # side 0, no team given: rebuilt from the first request
        while not battle.ended and battle.turn < 60:
            lines = battle.drain_player_log(0)
            battle.drain_player_log(1)
            player.feed(lines)
            if battle.needs_action(0):
                player.set_request(battle.request_json(0))
                decision = player.decide()
                assert decision is not None and decision.choice
                try:
                    battle.choose(0, decision.choice)
                except numbrion.ChoiceError as error:  # a hidden trap the request could not show
                    player.rejected(str(error.args[0]))
                    player.set_request(error.args[1])
                    battle.choose(0, player.decide().choice)
                source = decision.analysis["source"]
                sources[source] = sources.get(source, 0) + 1
                json.dumps(decision.analysis)  # the advisor sends it as JSON
            if battle.needs_action(1):
                battle.choose(1, "default")
    assert sources["search"] > 2 * sources["policy"]


def test_pairs_no_world_allows_are_never_chosen():
    a = np.array([[-0.9], [0.0]])
    x, _, q = solve([a], [np.array([True, False])], np.ones(1), np.array([0.1, 0.9]), [np.ones(1)], anchor=0.05,
                    opponent_anchor=0.1, iterations=50)
    assert q[1] == -2.0 and x[1] == 0.0 and int(np.argmax(x)) == 0


def _drive(bot, seed, teams, turn_hook):
    """Side 0 is a SearchPlayer fed only its player view; side 1 plays Showdown's default. ``turn_hook(battle,
    player)`` returns the choice text for side 0 (it may call the player). Returns the analysis sources."""
    battle = numbrion.Battle(seed, p1=teams[0], p2=teams[1], log=True)
    battle.start()
    player = bot.new_player("hook")
    sources = []
    while not battle.ended and battle.turn < 40:
        player.feed(battle.drain_player_log(0))
        battle.drain_player_log(1)
        if battle.needs_action(0):
            player.set_request(battle.request_json(0))
            choice, source = turn_hook(battle, player)
            battle.choose(0, choice)
            if source:
                sources.append(source)
        if battle.needs_action(1):
            battle.choose(1, "default")
    return sources


def test_recorded_defaults_and_resent_requests_keep_the_worlds(bot):
    teams = [line.strip() for line in gzip.open(POOL, "rt") if line.strip()][20:22]
    state = {"n": 0}

    def hook(battle, player):
        state["n"] += 1
        if state["n"] <= 3:  # the driver sent "default" for us (e.g. after two refusals)
            player.record_choice("default")
            return "default", None
        if state["n"] == 4:  # a refused choice: the server re-sends the same request without new lines
            player.decide()
            player.rejected("[Invalid choice] test")
            player.set_request(battle.request_json(0))
        decision = player.decide()
        return decision.choice, decision.analysis["source"]

    sources = _drive(bot, (7, 7, 7, 7), teams, hook)
    assert sources and sources.count("search") >= len(sources) - 1, sources
