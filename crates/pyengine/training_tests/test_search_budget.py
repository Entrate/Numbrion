"""Budget control with a fake serving clock, including bounded and unbounded tails."""
from types import SimpleNamespace

import numpy as np
import pytest

from training.search2_types import Depth2Config, HalvingRound, Layer1, Stage
from training.search_budget import BudgetController, PRIORS


class Clock:
    def __init__(self):
        self.now = 0.0

    def advance(self, ms):
        self.now += ms / 1000


class World:
    def __init__(self, remaining=6):
        self.remaining = remaining

    def request_json(self, side):
        return {"side": {"pokemon": [{"condition": "100/100" if n < self.remaining else "0 fnt"}
                                     for n in range(6)]}}


def layer(worlds=32, confident=False, remaining=6):
    return Layer1(0, [World(remaining) for _ in range(worlds)], (), np.zeros((5, 2)), np.ones(5) / 5,
                  [], [], [], [], np.array([.96, .01, .01, .01, .01]) if confident else np.ones(5) / 5,
                  [], np.array([.8, .1, .0, -.1, -.2]) if confident else np.linspace(0, .05, 5), [])


def config():
    return SimpleNamespace(worlds=4, max_worlds=32, ours=12, theirs=8, seeds=2)


def controller(**options):
    return BudgetController.from_config(options, 128, 3, .075)


def work(cfg, l1):
    """Fake core: nested expansion, two policies per leaf, cell grid, values, grouped solve."""
    last_worlds = last_chance = 0
    for index, r in enumerate(cfg.rounds):
        roots = min(len(l1.ours), cfg.shortlist + 1) if index == 0 else min(len(l1.ours), r.keep)
        samples = r.worlds * r.chance - last_worlds * last_chance
        leaves = roots * (cfg.theirs_top + cfg.danger) * samples
        yield "expand", leaves
        yield "leaf_policy", 2 * leaves
        yield "grid2", leaves * cfg.ours * cfg.theirs * cfg.seeds
        yield "value2", leaves * cfg.ours * cfg.theirs * cfg.seeds
        yield "solve2", roots * (cfg.theirs_top + cfg.danger) * r.chance
        last_worlds, last_chance = r.worlds, r.chance


def simulate(decisions, seed=4, unbounded=False):
    clock, rng = Clock(), np.random.default_rng(seed)
    priors = {name: {"fixed_ms": f, "unit_ms": u} for name, (f, u) in PRIORS.items()}
    b = controller(priors=priors, bound_factor=4, chunk_ms=400)
    depth = Depth2Config(enabled=True, replacement_steps=0)
    totals, overruns, violations, spikes = [], 0, 0, 0
    for _ in range(decisions):
        began = clock.now
        plan = b.plan(config(), depth, 3000, clock.now + 5.5, clock.now)
        observed = []

        def run(stage, units):
            nonlocal violations, spikes
            while units > 0:
                # A serving core may split further; find the largest call admitted by the controller.
                low, high = 0, units
                while low < high:
                    mid = (low + high + 1) // 2
                    if b.next_chunk_ok(stage, mid, clock.now, plan.deadline):
                        low = mid
                    else:
                        high = mid - 1
                if not low:
                    return False
                fixed, unit = PRIORS[stage]
                multiplier = float(rng.uniform(.85, 1.15))
                if rng.random() < .01:
                    multiplier = 3
                    spikes += 1
                mean = fixed + unit * low
                bound = b.models[stage].bound(low, b.bound_factor, b.deviation_k)
                actual = mean * multiplier
                if unbounded and rng.random() < .002:
                    actual = bound + 6000  # explicitly violate the safety premise
                violations += int(actual > bound + 1e-6)
                observed.append(Stage(stage, low, actual))
                clock.advance(actual)
                units -= low
            return True

        n = plan.worlds
        for s, units in [("root_policy", 1), ("worlds", n), ("opp_policy", n),
                         ("grid1", n * 192), ("value1", n * 192)]:
            if not run(s, units):
                break
        else:
            l1 = layer(n)
            cfg2 = b.plan_depth2(plan, l1, clock.now)
            if cfg2:
                for stage, units in work(cfg2, l1):
                    if not run(stage, units):
                        break
        clock.advance(50)  # solve, analysis and send, covered by 150 ms margin
        total = (clock.now - began) * 1000
        overruns += int(clock.now > plan.deadline + 1e-8)
        b.observe(observed, total)
        totals.append(total)
    return np.array(totals), overruns, violations, spikes, b


def test_long_run_mean_and_ten_thousand_bounded_tail_decisions():
    totals, overruns, violations, spikes, b = simulate(10_000)
    assert spikes > 100
    assert violations == 0
    assert overruns == 0
    assert 2700 <= totals[500:].mean() <= 3300, (totals[500:].mean(), b.stats())


def test_unbounded_chunks_explain_and_count_overruns():
    totals, overruns, violations, _, b = simulate(1000, unbounded=True)
    assert violations > 0 and overruns > 0
    assert overruns <= violations
    assert sum(s["bound_violations"] for s in b.stats()["stages"].values()) > 0
    assert totals.max() > 5500


def test_larger_networks_get_smaller_plans():
    sizes = []
    for width, layers in [(128, 3), (192, 4), (256, 6)]:
        b = BudgetController.from_config({}, width, layers, .075)
        plan = b.plan(config(), Depth2Config(enabled=True), 3000, 5.5, 0)
        cfg2 = b.plan_depth2(plan, layer(plan.worlds), .52)
        sizes.append(0 if cfg2 is None else b._sizes(cfg2, layer(plan.worlds))["grid2"])
    assert sizes[0] > sizes[1] > sizes[2], sizes


def test_measured_slow_network_shrinks_without_architecture_change():
    b = controller()
    cfg2 = Depth2Config(enabled=True)
    p = b.plan(config(), cfg2, 3000, 5.5, 0)
    fast = b.plan_depth2(p, layer(p.worlds), .52)
    for _ in range(30):
        for n in (1, 8, 32):
            b.observe([Stage("leaf_policy", n, 4 + 12 * n)], 3000)
    p = b.plan(config(), cfg2, 3000, 5.5, 0)
    slow = b.plan_depth2(p, layer(p.worlds), .52)
    assert fast is not None
    assert slow is None or b._sizes(slow, layer())["grid2"] < b._sizes(fast, layer())["grid2"]
    assert b.stats()["preset"] == "large"


def test_fixed_is_exact_and_does_not_mutate_config():
    b = controller(mode="fixed")
    cfg2 = Depth2Config(enabled=True, rounds=(HalvingRound(3, 7, 3),))
    cfg = config()
    plan = b.plan(cfg, cfg2, 10, 5.5, 0)
    assert plan.worlds == cfg.worlds
    assert b.plan_depth2(plan, layer(32), 1) is cfg2
    b.observe([], 100)
    assert b.stats()["scale"] == 1
    assert cfg2.rounds[0].worlds == 7


def test_depth2_skips_with_no_time_and_when_disabled():
    b = controller()
    plan = b.plan(config(), Depth2Config(enabled=True), 3000, 1, 0)
    assert b.plan_depth2(plan, layer(), .86) is None
    assert b.plan_depth2(b.plan(config(), Depth2Config(), 3000, 5.5, 0), layer(), .5) is None
    assert b.plan(config(), None, 3000, -.1, 0).worlds == 0


def test_difficulty_floor_keeps_known_threat_check_and_endgame_budget():
    b = controller()
    ordinary, easy, endgame = layer(), layer(confident=True), layer(remaining=2)
    assert b._difficulty(easy) < b._difficulty(ordinary) < b._difficulty(endgame)
    easy.difficulty = {"known_threat": True, "factor": 0}
    assert b._difficulty(easy) >= 1
    plan = b.plan(config(), Depth2Config(enabled=True), 3000, 5.5, 0)
    assert b.plan_depth2(plan, easy, .52).danger == 1


def test_bounds_margin_chunk_cap_and_external_deadline():
    b = controller(max_ms=1000)
    p = b.plan(config(), None, None, 5.5, 0)
    assert p.deadline == 1
    assert not b.next_chunk_ok("value2", 100_000, 0, p.deadline)
    assert not b.next_chunk_ok("root_policy", 1, .99, p.deadline)
    assert b.next_chunk_ok("root_policy", 1, 0, p.deadline)
    with pytest.raises(ValueError, match="unknown"):
        b.next_chunk_ok("typo", 1, 0, None)


def test_fit_fixed_and_unit_costs_from_varying_batches():
    b = controller()
    for _ in range(50):
        for n in [1, 4, 16, 64, 256]:
            b.observe([Stage("leaf_policy", n, 7 + 2.5 * n)], 3000)
    fitted = b.stats()["stages"]["leaf_policy"]
    assert fitted["fixed_ms"] == pytest.approx(7, abs=.2)
    assert fitted["unit_ms"] == pytest.approx(2.5, rel=.01)


def test_reduced_rounds_are_nested_and_replies_shrink_before_leaves():
    b = controller()
    ladder = list(b._ladder(Depth2Config(enabled=True), 32))
    for cfg in ladder:
        assert all(a.worlds <= z.worlds and a.chance <= z.chance for a, z in zip(cfg.rounds, cfg.rounds[1:]))
    for before, after in zip(ladder, ladder[1:]):
        if (after.ours, after.theirs, after.seeds) != (before.ours, before.theirs, before.seeds):
            assert len(after.rounds) == 1 and after.rounds[0].worlds == after.rounds[0].chance == 1
            assert after.theirs_top == 1 and after.danger == 0


@pytest.mark.parametrize("options", [{"target_ms": -1}, {"mode": "other"}, {"chunk_ms": 0},
                                     {"margin_ms": float("nan")}, {"mean_alpha": 2}])
def test_invalid_budget(options):
    with pytest.raises(ValueError):
        controller(**options)
