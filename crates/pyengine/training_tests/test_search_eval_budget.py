"""Budget propagation and decision-weighted world counts in the search evaluator."""
from types import SimpleNamespace

import pytest

import search_eval


def game(worlds_count=0, worlds_mean=None, opponent_worlds_count=0, opponent_worlds_mean=None):
    return dict(score=1.0, ended=True, rejected=0, stats={}, worlds_count=worlds_count, worlds_mean=worlds_mean,
                opponent_worlds_count=opponent_worlds_count, opponent_worlds_mean=opponent_worlds_mean)


def test_worlds_means_weight_search_decisions_and_allow_no_search():
    results = [dict(games=[game(1, 2, 2, 8), game(3, 6, 1, 2), game()])]
    assert search_eval.worlds_means(results) == dict(worlds_mean=5.0, opponent_worlds_mean=6.0)
    assert search_eval.worlds_means([dict(games=[game()])]) == dict(worlds_mean=None, opponent_worlds_mean=None)


@pytest.mark.parametrize("budget_ms", [None, 1500.5])
def test_play_game_sets_budget_on_every_attempt_and_counts_only_search(monkeypatch, budget_ms):
    class Player:
        stats = {}

        def __init__(self, analyses):
            self.analyses = iter(analyses)
            self.budgets = []

        def feed(self, lines):
            pass

        def set_request(self, request):
            pass

        def rejected(self, error):
            pass

        def decide(self):
            self.budgets.append(self.budget_ms)
            self.budget_ms = -1  # the next call must restore the per-decision budget, including retries
            return SimpleNamespace(a0=0, a1=0, analysis=next(self.analyses))

    class Battle:
        ended, turn, tie, winner = False, 1, False, 0

        def __init__(self, *args, **kwargs):
            self.rejected = False
            self.accepted = 0

        def start(self):
            pass

        def drain_player_log(self, side):
            return []

        def needs_action(self, side):
            return True

        def request_json(self, side):
            return "{}"

        def choose_action(self, side, a0, a1):
            if side == 0 and not self.rejected:
                self.rejected = True
                raise search_eval.numbrion.ChoiceError("retry")
            self.accepted += 1
            self.ended = self.accepted == 4

    player = Player([dict(source="search", worlds=2), dict(source="search", worlds=6), dict(source="policy")])
    opponent_player = Player([dict(source="policy"), dict(source="search", worlds=10)])
    bot = SimpleNamespace(new_player=lambda name: player)
    opponent_bot = SimpleNamespace(new_player=lambda name: opponent_player)
    factory = lambda side, team: search_eval.SearchOpponent(opponent_bot, side, team, False, budget_ms)
    monkeypatch.setattr(search_eval.numbrion, "Battle", Battle)
    result = search_eval.play_game(bot, factory, "a", "b", (1, 2, 3, 4), 0, False, budget_ms=budget_ms)
    assert player.budgets == [budget_ms] * 3
    assert opponent_player.budgets == [budget_ms] * 2
    assert result["rejected"] == 1
    assert (result["worlds_count"], result["worlds_mean"]) == (2, 4.0)
    assert (result["opponent_worlds_count"], result["opponent_worlds_mean"]) == (1, 10.0)


@pytest.mark.parametrize("flags, expected", [
    ([], (None, None)),
    (["--budget-ms", "1500.5"], (1500.5, 1500.5)),
    (["--budget-ms", "1500", "--opponent-budget-ms", "750"], (1500, 750)),
    (["--opponent-budget-ms", "750"], (None, 750)),
])
def test_cli_budget_defaults_and_override(monkeypatch, tmp_path, flags, expected):
    teams = tmp_path / "teams.txt"
    teams.write_text("a\nb\n")
    calls = []

    def bot(**kwargs):
        return SimpleNamespace(net=None, dex=SimpleNamespace(path="unused"),
                               new_player=lambda name: SimpleNamespace(), describe=lambda: {})

    def play_game(bot, factory, t1, t2, seed, side, engine_mask, *, budget_ms):
        opponent = factory(1 - side, t2)
        calls.append((budget_ms, opponent.budget_ms))
        return game(1, 4, 1, 6)

    monkeypatch.setattr(search_eval, "SearchBot", bot)
    monkeypatch.setattr(search_eval, "play_game", play_game)
    monkeypatch.setattr(search_eval, "RUN_FLAG", tmp_path / "inactive")
    monkeypatch.setattr(search_eval.torch, "set_num_threads", lambda n: None)
    monkeypatch.setattr(search_eval.sys, "argv", ["search_eval.py", "--checkpoint", "unused", "--opponent", "search",
                                               "--pairs", "1", "--pool", str(teams),
                                               "--out", str(tmp_path / "out.json"), *flags])
    search_eval.main()
    assert calls == [expected, expected]
