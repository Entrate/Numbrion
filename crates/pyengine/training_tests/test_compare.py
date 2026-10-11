"""compare.py's pure parts (keep/drop rule, mirrored-pair interval, cache key) and play's per-pair outcomes."""
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest

import compare
import evaluate
from training.guards import MoveGuards


def result_from_pairs(first, mirrored):
    """A play() result from a's score in both games of each mirrored pair (pair i is env i, battle 1)."""
    games = first + mirrored
    sums = [x + y for x, y in zip(first, mirrored)]
    return dict(games=len(games), score=sum(games) / len(games), wins=sum(x == 1 for x in games),
                ties=sum(x == 0.5 for x in games), pair_sweeps=sum(s == 2 for s in sums),
                pair_splits=sum(s == 1 for s in sums), pair_losses=sum(s == 0 for s in sums),
                pairs=dict(env=list(range(len(sums))), battle=[1] * len(sums), first=first, mirrored=mirrored))


def test_pair_interval_matches_direct_pair_statistics():
    rng = np.random.default_rng(3)
    first = [float(x) for x in rng.choice([0, 0.5, 1], size=500, p=[0.4, 0.1, 0.5])]
    mirrored = [float(x) for x in rng.choice([0, 0.5, 1], size=500, p=[0.45, 0.1, 0.45])]
    result = result_from_pairs(first, mirrored)
    means = (np.array(first) + np.array(mirrored)) / 2
    error = 1.96 * means.std(ddof=1) / math.sqrt(len(means))
    assert compare.pair_interval(result) == pytest.approx([means.mean() - error, means.mean() + error])
    # Pairs carry the information: a sweep/loss-only matchup is wider than the same score from splits only.
    swept = compare.pair_interval(result_from_pairs([1.0, 0.0] * 50, [1.0, 0.0] * 50))
    split = compare.pair_interval(result_from_pairs([1.0, 0.0] * 50, [0.0, 1.0] * 50))
    assert split == [0.5, 0.5] and swept[0] < 0.45 and swept[1] > 0.55


def test_pair_interval_extremes():
    bound = 1.96 ** 2 / (100 + 1.96 ** 2)
    assert compare.pair_interval(result_from_pairs([1.0] * 100, [1.0] * 100)) == pytest.approx([1 - bound, 1])
    assert compare.pair_interval(result_from_pairs([0.0] * 100, [0.0] * 100)) == pytest.approx([0, bound])


def panel(smart, heuristic, ref, pairs=50):
    """Panel results whose scores are exact: ``x`` points out of 2 * pairs games for each opponent."""
    def result(points):
        first = [1.0] * min(points, pairs) + [0.0] * (pairs - min(points, pairs))
        rest = points - min(points, pairs)
        return result_from_pairs(first, [1.0] * rest + [0.0] * (pairs - rest))
    return dict(smart=result(smart), heuristic=result(heuristic), ref=result(ref))


def test_decide_keeps_at_the_bar_and_at_the_largest_allowed_drop():
    h2h = panel(53, 0, 0)["smart"]  # 0.53 exactly
    best = panel(53, 60, 50)
    decision = compare.decide(h2h, panel(50, 70, 47), best, bar=0.53, panel_drop=0.03)
    assert decision["verdict"] == "KEEP" and decision["h2h_ok"] and not decision["reasons"]
    assert [row["ok"] for row in decision["panel"]] == [True, True, True]
    assert decision["panel"][0]["difference"] == pytest.approx(-0.03)
    assert decision["h2h_95pct_interval"] == compare.pair_interval(h2h)


def test_decide_drops_below_the_bar_or_on_a_panel_drop():
    best = panel(53, 60, 50)
    below = compare.decide(panel(52, 0, 0)["smart"], panel(60, 70, 60), best, bar=0.53, panel_drop=0.03)
    assert below["verdict"] == "DROP" and not below["h2h_ok"] and len(below["reasons"]) == 1
    dropped = compare.decide(panel(60, 0, 0)["smart"], panel(60, 56, 50), best, bar=0.53, panel_drop=0.03)
    assert dropped["verdict"] == "DROP" and dropped["h2h_ok"]
    assert [row["ok"] for row in dropped["panel"]] == [True, False, True]
    assert "heuristic" in dropped["reasons"][0]


def test_difference_interval_pairs_by_env_and_battle():
    a = result_from_pairs([1.0, 0.0, 1.0], [0.0, 0.0, 1.0])
    b = result_from_pairs([1.0, 0.0, 1.0], [0.0, 0.0, 1.0])
    assert compare.difference_interval(a, b) == [0.0, 0.0]
    shuffled = dict(b, pairs={k: v[::-1] for k, v in b["pairs"].items()})
    assert compare.difference_interval(a, shuffled) == [0.0, 0.0]
    b["pairs"]["first"][2] = 0.0  # one pair half a point worse: mean difference 1/6
    low, high = compare.difference_interval(a, b)
    assert low < 1 / 6 < high
    assert compare.difference_interval(a, {k: v for k, v in b.items() if k != "pairs"}) is None


def test_cache_key_covers_every_input():
    base = dict(checkpoint="c" * 64, opponent="smart", temperature=0.5, seed=950, rounds=20, pool="p" * 64,
                envs=50, workers=4, guards=MoveGuards(), device="mps:0")
    key = compare.cache_key(**base)
    assert key == compare.cache_key(**base)
    assert compare.cache_key(**dict(base, temperature=1)) == compare.cache_key(**dict(base, temperature=1.0))
    changes = dict(checkpoint="d" * 64, opponent="policy:" + "e" * 64 + "@0.5", temperature=1.0, seed=953,
                   rounds=40, pool="q" * 64, envs=25, workers=2, guards=MoveGuards(repeat_protect=False),
                   device="cpu")
    keys = {compare.cache_key(**dict(base, **{name: value})) for name, value in changes.items()}
    assert len(keys) == len(changes) and key not in keys
    assert key != compare.cache_key(**base, action_seed=78)


def test_play_reports_per_pair_outcomes_only_when_asked(monkeypatch, tmp_path):
    winners = np.array([[0, 1], [-1, 0], [0, 0], [1, 1]])  # [env, battle]; envs 2-3 mirror envs 0-1

    class StubVecEnv:
        def __init__(self, n, pools, **kwargs):
            assert n == 4 and kwargs["mirror"]
            self.obs = dict(needs_action=np.ones((n, 2), dtype=bool),
                            mask0=np.ones((n, 2, evaluate.N_ACTIONS), dtype=bool),
                            battle_id=np.ones(n, dtype=np.int64), turn=np.ones(n, dtype=np.int32),
                            done=np.zeros(n, dtype=bool), winner=np.full(n, -1))
            self.steps = 0

        def __enter__(self):
            return self

        def __exit__(self, *args):
            pass

        def reset(self):
            return self.obs

        def mask_slot1(self, a0):
            return self.obs["mask0"]

        def step(self, actions):
            self.obs["done"][:] = True
            self.obs["winner"][:] = winners[:, self.steps]
            self.steps += 1
            return self.obs

    class StubAgent:
        name = "stub"

        def first(self, obs, rows, mask):
            return np.full(len(rows), evaluate.PASS, dtype=np.int32)

        def second(self, rows, mask):
            return np.full(len(rows), evaluate.PASS, dtype=np.int32)

    monkeypatch.setattr(evaluate, "VecEnv", StubVecEnv)
    plain, *_ = evaluate.play(StubAgent(), StubAgent(), tmp_path / "unused.txt", 2, 2, 7, 2)
    result, *_ = evaluate.play(StubAgent(), StubAgent(), tmp_path / "unused.txt", 2, 2, 7, 2, pairs=True)
    assert "pairs" not in plain and {k: v for k, v in result.items() if k != "pairs"} == plain
    assert result["pairs"] == dict(env=[0, 0, 1, 1], battle=[1, 2, 1, 2], first=[1.0, 0.0, 0.5, 1.0],
                                   mirrored=[0.0, 0.0, 1.0, 1.0])
    assert (result["pair_sweeps"], result["pair_splits"], result["pair_losses"]) == (1, 1, 1)
    assert result["score"] == pytest.approx(4.5 / 8)
    assert compare.pair_interval(result)[0] < result["score"]
