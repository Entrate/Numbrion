"""training.search2: the two-ply search's mechanics (a random-weight model: this checks mechanics, not strength).

Directed battles are built from hand-written teams; the searched games use only the player's view, as
test_search.py's do."""
from __future__ import annotations

import gzip
import json
import time
import warnings
from pathlib import Path

import numpy as np
import pytest
import torch

numbrion = pytest.importorskip("numbrion")
if not (numbrion.SEARCH and hasattr(numbrion, "SearchGrid")):
    pytest.skip("needs a search wheel with the fused grid", allow_module_level=True)

from training import protocol as P  # noqa: E402
from training import search2  # noqa: E402
from training.dex import load_dex  # noqa: E402
from training.model import Model  # noqa: E402
from training.search import SearchBot, solve  # noqa: E402
from training.search2 import Depth2Config, Layer1, StageClock, _Deepen, _Leaf, expand  # noqa: E402
from training.search2_types import Plan  # noqa: E402

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "crates/pyengine/testdata/pool-400.txt.gz"
SMALL = dict(enabled=True, shortlist=3, rounds=[dict(keep=3, worlds=2, chance=1), dict(keep=2, worlds=3, chance=2)],
             theirs_top=2, danger=1, ours=4, theirs=3, seeds=1)
MON = "{name}||Leftovers|{ability}|{moves}||85,85,85,85,85,85|{gender}|||80|,,,,,Normal"


def mon(name, moves, ability, gender="M"):
    return MON.format(name=name, moves=",".join(moves), ability=ability, gender=gender)


FILLER = [mon("Snorlax", ["bodyslam", "protect", "curse", "rest"], "Thick Fat"),
          mon("Blissey", ["hypervoice", "protect", "softboiled", "toxic"], "Healer", "F"),
          mon("Skarmory", ["bravebird", "protect", "roost", "spikes"], "Sturdy"),
          mon("Garchomp", ["earthquake", "protect", "dragonclaw", "swordsdance"], "Rough Skin")]


def teams():
    return [line.strip() for line in gzip.open(POOL, "rt") if line.strip()]


@pytest.fixture(scope="module")
def checkpoint(tmp_path_factory):
    torch.manual_seed(0)
    path = tmp_path_factory.mktemp("search2") / "random.pt"
    torch.save({"model": Model(load_dex()).state_dict(), "width": 128, "layers": 3}, path)
    return path


def make_bot(checkpoint, **depth2):
    cfg = dict(SMALL, **depth2)
    # anchor 0.05 (the pre-2026-10-16 default) so the anchored pick and sampled play are exercised; the play default
    # is now 0 (no nudge toward the policy), covered by the anchor-0 fallback test.
    return SearchBot(checkpoint=str(checkpoint), search={"worlds": 3, "ours": 4, "theirs": 3, "seeds": 1, "threads": 2,
                                                         "iterations": 60, "anchor": 0.05, "depth2": cfg},
                     pools=[str(POOL)], device="cpu")


@pytest.fixture(scope="module")
def bot(checkpoint):
    return make_bot(checkpoint)


def play(bot, seed, game_seed=(3, 1, 4, 1), decisions=6, hook=None, team_index=0):
    """Side 0 is a seeded SearchPlayer fed its player view; side 1 plays Showdown's default. Returns the decisions."""
    pool = teams()
    battle = numbrion.Battle(game_seed, p1=pool[team_index], p2=pool[team_index + 1], log=True)
    battle.start()
    player = bot.new_player("t", seed=seed)
    out = []
    while not battle.ended and battle.turn < 30 and len(out) < decisions:
        player.feed(battle.drain_player_log(0))
        battle.drain_player_log(1)
        if battle.needs_action(0):
            player.set_request(battle.request_json(0))
            if hook:
                hook(player)
            decision = player.decide()
            out.append(decision)
            try:
                battle.choose(0, decision.choice)
            except numbrion.ChoiceError as error:
                player.rejected(str(error.args[0]))
                player.set_request(error.args[1])
                battle.choose(0, player.decide().choice)
        if battle.needs_action(1):
            battle.choose(1, "default")
    return out, player


def strip_timing(d2: dict) -> dict:
    return {k: v for k, v in d2.items() if k not in ("timing", "total_ms")}


# ---- solver --------------------------------------------------------------------------------------------------------

def test_solve_groups_matches_solve_per_game():
    rng = np.random.default_rng(0)
    G, W, N, M = 4, 3, 5, 4
    A = rng.uniform(-1, 1, (G, W, N, M))
    valid = rng.random((G, W, N)) > 0.2
    valid[:, :, 0] = True
    wmask = np.ones((G, W), dtype=bool)
    wmask[1, 2] = False
    ours = rng.random((G, N))
    ours[2, 4] = 0.0  # a padding pair
    theirs = rng.random((G, W, M))
    theirs[3, :, 3] = 0.0  # a padding reply
    for opp_anchor in (0.1, 0.0):
        x, ys, q = search2.solve_groups(A, valid, wmask, ours, theirs, 0.05, opp_anchor, 80)
        for g in range(G):
            w = np.flatnonzero(wmask[g])
            n = N - 1 if g == 2 else N
            m = M - 1 if g == 3 else M
            tp = [theirs[g, k, :m] / theirs[g, k, :m].sum() for k in w]
            x1, ys1, q1 = solve([A[g, k, :n, :m] for k in w], [valid[g, k, :n] for k in w], np.ones(len(w)),
                                ours[g, :n] / ours[g, :n].sum(), tp, 0.05, opp_anchor, 80)
            assert np.allclose(x[g, :n], x1, atol=1e-9)
            assert np.allclose(q[g, :n], q1, atol=1e-9)
            for r, k in enumerate(w):
                assert np.allclose(ys[g, k, :m], ys1[r], atol=1e-9)


# ---- whole decisions -------------------------------------------------------------------------------------------------

def test_same_seed_gives_the_same_result(checkpoint):
    runs = []
    for _ in range(2):
        bot = make_bot(checkpoint)
        decisions, _ = play(bot, seed=11)
        runs.append([(d.choice, strip_timing(d.analysis["depth2"]) if "depth2" in d.analysis else None)
                     for d in decisions])
    assert runs[0] == runs[1]
    assert any(d2 and d2["rounds"] == 2 for _, d2 in runs[0])


def test_decisions_carry_a_serialisable_compact_depth2_block(bot):
    decisions, player = play(bot, seed=5, decisions=8)
    blocks = [d.analysis["depth2"] for d in decisions if "depth2" in d.analysis]
    assert blocks
    for d, block in ((d, d.analysis.get("depth2")) for d in decisions):
        text = json.dumps(d.analysis)  # the advisor and the recorder send it as JSON
        if block is None:
            continue
        assert len(json.dumps(block)) < 5000
        if block["rounds"]:
            assert block["chosen"] == d.choice
            layer1 = max(block["candidates"], key=lambda c: c["x"])["choice"]
            assert block["changed"] == (block["chosen"] != layer1)
            assert set(block["timing"]) >= {"expand", "leaf_policy", "grid2", "value2", "solve2"}
            assert block["cells"]["expand"] > 0 and block["leaves"]["total"] > 0
            for c in block["candidates"]:
                assert len(c["q2"]) == block["rounds"]
        assert "Infinity" not in text and "NaN" not in text
    assert player.stats.get("depth2", 0) >= 1 and not player.stats.get("depth2_errors")


@pytest.mark.parametrize("variant", [dict(info="independent"), dict(backup="resolve"), dict(leaf_value="argmax"),
                                     dict(replacement_steps=0)])
def test_ablation_variants_run(checkpoint, variant):
    decisions, player = play(make_bot(checkpoint, **variant), seed=2, decisions=3)
    assert any(d.analysis.get("depth2", {}).get("rounds") == 2 for d in decisions)
    assert not player.stats.get("depth2_errors"), [d.analysis.get("depth2", {}).get("skipped") for d in decisions]


def test_layer2_that_adds_nothing_picks_the_anchored_best_response_to_layer1_values(checkpoint, monkeypatch):
    """With every leaf valued at its layer-1 payoff (V2 = A1), the corrections vanish: Q2 = Q1 and the pick is
    argmax(log prior + Q1 / anchor) over the shortlist (halving never drops it: it ranks by the same score)."""
    def no_op_layer2(self, keys):
        for key in keys:
            k, i, j, _ = key
            self.leaves[key].value = float(self.l1.payoffs[k][i][j])
        return True

    runs = []
    original = _Deepen.run

    def run(self):
        info = original(self)
        runs.append((self, info))
        return info

    monkeypatch.setattr(_Deepen, "layer2", no_op_layer2)
    monkeypatch.setattr(_Deepen, "run", run)
    decisions, _ = play(make_bot(checkpoint), seed=7, decisions=8)
    checked = 0
    for self, info in runs:
        if not info["completed"]:
            continue
        q1 = np.clip(np.asarray(self.l1.q), -1, 1)
        anchored = {i: np.log(self.l1.our_prior[i]) + q1[i] / self.cfg.anchor for i in info["shortlist"]}
        assert info["chosen"] == max(anchored, key=anchored.get)
        for r in info["rounds"]:
            assert all(abs(r["q2"][i] - q1[i]) < 1e-9 for i in r["survivors"])
        checked += 1
    assert checked >= 2
    for d in decisions:
        block = d.analysis.get("depth2")
        if block and block["rounds"]:
            assert d.choice == block["chosen"]


def run_synthetic(bot, l1, leaf_value, cfg=None, pick="anchored", **depth2):
    """``_Deepen.run`` on a synthetic Layer1 whose leaves get ``leaf_value(k, i, j)`` (no engine work)."""
    run = _Deepen(bot, cfg or bot.search, Depth2Config(enabled=True, pick=pick, **depth2), l1, StageClock(),
                  np.random.default_rng(0), pick, ("", ""), None)
    run.grow = lambda keys: True

    def layer2(keys):
        for key in keys:
            run.leaves[key] = _Leaf(key=key, value=leaf_value(*key[:3]))
        return True

    run.layer2 = layer2
    return run.run()


def test_a_certain_win_never_loses_to_a_worse_move_with_a_large_correction(bot):
    """Candidate 0 wins for sure (A1 = V2 = 1, Q1 = Q2 = 1). The others are worse at layer 1 and get large positive
    corrections (V2 = 0.6 against A1 = -0.5) but stay below 1, and the prior strongly prefers them."""
    l1 = synthetic_layer1(n_ours=3, n_worlds=2, m=3)
    l1.payoffs = [np.array([[1.0] * 3, [-0.5] * 3, [-0.5] * 3]) for _ in range(2)]
    l1.q = np.array([1.0, -0.5, -0.5])
    l1.x = np.array([0.98, 0.01, 0.01])
    l1.our_prior = np.array([0.001, 0.9, 0.099])
    for pick in ("anchored", "q"):
        info = run_synthetic(bot, l1, lambda k, i, j: 1.0 if i == 0 else 0.6, pick=pick)
        assert info["completed"] == 2 and info["chosen"] == 0
        assert info["rounds"][-1]["q2"][0] == 1.0 and abs(info["rounds"][0]["q2"][1] - 0.6) < 1e-9
    # Corrections are averaged over the deepened worlds only: Q1 + correction is clipped to [-1, 1].
    l1.q = np.array([0.9, -0.5, -0.5])
    info = run_synthetic(bot, l1, lambda k, i, j: 1.0 if i == 0 else 0.6)
    assert all(-1.0 <= v <= 1.0 for r in info["rounds"] for v in r["q2"].values())


def test_anchor_zero_falls_back_to_argmax_q2(bot):
    from dataclasses import replace

    l1 = synthetic_layer1(n_ours=3, n_worlds=2, m=3)
    l1.our_prior = np.array([0.98, 0.01, 0.01])
    info = run_synthetic(bot, l1, lambda k, i, j: 0.5 if i == 2 else -0.5, cfg=replace(bot.search, anchor=0.0))
    assert info["chosen"] == 2


def test_refused_chunks_keep_the_layer1_decision_and_post_it_as_the_fallback(checkpoint):
    class Refuse:
        """A budget controller that plans the configured sizes, admits layer 1, and lets ``allow`` layer-2 chunk
        requests through."""
        def __init__(self, allow):
            self.allow, self.calls, self.observed, self.plans = allow, 0, [], []

        def plan(self, cfg, cfg2, budget_ms, deadline, now):
            self.plans.append(Plan(worlds=cfg.worlds, depth2=cfg2, budget_ms=0.0, deadline=deadline))
            return self.plans[-1]

        def plan_depth2(self, plan, layer1, now):
            assert plan is self.plans[-1] and plan.started is not None and layer1.difficulty is not None
            return plan.depth2

        def next_chunk_ok(self, stage, units, now, deadline):
            if stage in ("root_policy", "opp_policy", "grid1", "value1"):
                return True
            self.calls += 1
            return self.calls <= self.allow

        def observe(self, stages, total_ms):
            self.observed.append((stages, total_ms))

    bot = make_bot(checkpoint)
    bot.budget = Refuse(allow=0)
    tokens = iter(range(100))

    def hook(player):
        player.decision_token = next(tokens)

    decisions, player = play(bot, seed=3, decisions=4, hook=hook)
    searched = [d for d in decisions if d.analysis.get("source") == "search"]
    assert searched
    for d in searched:
        block = d.analysis["depth2"]
        assert block["rounds"] == 0 and block["chosen"] is None and "refused" in block["skipped"]
        top = max(d.analysis["candidates"], key=lambda c: c["weight"])
        assert d.choice == top["choice"]
    token, fallback = player.fallback
    assert fallback.choice == decisions[-1].choice and "layer-1" in fallback.analysis["note"]
    stage_names = {s.name for stages, _ in bot.budget.observed for s in stages}
    assert {"root_policy", "worlds", "grid1", "value1"} <= stage_names
    # One round's worth of chunks: the first round completes, the second is refused.
    bot.budget = Refuse(allow=10 ** 6)
    full, _ = play(bot, seed=3, decisions=1)
    chunks = bot.budget.calls
    bot.budget = Refuse(allow=chunks // 2)
    partial, _ = play(bot, seed=3, decisions=1)
    assert full[0].analysis["depth2"]["rounds"] == 2
    assert partial[0].analysis["depth2"]["rounds"] in (0, 1) and partial[0].analysis["depth2"]["skipped"]


def test_stage_clock_refuses_past_the_deadline():
    clock = StageClock(None, time.monotonic() + 0.05)
    assert not clock.ok("grid2", 1000) and "grid2" in clock.refused
    assert StageClock(None, None).ok("grid2", 10 ** 9)
    clock = StageClock(None, time.monotonic() + 60.0)
    assert clock.ok("grid2", 1000)
    with clock.stage("grid2", 100):
        pass
    assert clock.stages[0].name == "grid2" and clock.stages[0].units == 100 and "grid2" in clock.costs


# ---- expansion -----------------------------------------------------------------------------------------------------

def sampled_worlds(bot, turns=3):
    """A player after a few turns of a pool battle, and worlds sampled from its view."""
    pool = teams()
    battle = numbrion.Battle((5, 6, 7, 8), p1=pool[4], p2=pool[5], log=True)
    battle.start()
    player = bot.new_player("w", seed=1)
    for _ in range(turns):
        player.feed(battle.drain_player_log(0))
        battle.drain_player_log(1)
        if battle.needs_action(0):
            player.set_request(battle.request_json(0))
            battle.choose(0, player.decide().choice)
        if battle.needs_action(1):
            battle.choose(1, "default")
    player.feed(battle.drain_player_log(0))
    player.set_request(battle.request_json(0))
    player._sync(json.loads(player.request_json), 0)
    worlds = player.builder.worlds(3, 9, log=True)
    states = (player.battle.encoder.snapshot(0), player.battle.encoder.snapshot(1))
    return worlds, states


def test_expanded_leaf_observation_equals_feeding_the_lines_to_encode_states(bot):
    worlds, states = sampled_worlds(bot)
    actions, world_idx = [], []
    for k, w in enumerate(worlds):
        p0 = np.argwhere(w.legal_mask(0)[1])[:6]
        p1 = np.argwhere(w.legal_mask(1)[1])[:2]
        for a in p0:
            for b in p1:
                actions.append([a, b])
                world_idx.append(k)
    actions = np.asarray(actions, dtype=np.int32)
    world_idx = np.asarray(world_idx)
    seeds = np.tile(np.array([9, 8, 7, 6]), (len(actions), 1))
    exp = expand(bot, worlds, actions, world_idx, seeds, states, 0)
    ref = bot.resources().executor.run(worlds, actions, world=world_idx, seeds=seeds, logs=True, requests=True)
    grid = bot.resources().grid
    checked = 0
    for c in range(len(actions)):
        assert exp.ended[c] == ref["ended"][c]
        if exp.ended[c]:
            assert exp.leaves[c] is None and exp.trackers[c] is None
            continue
        for s in (0, 1):
            request = ref["requests"][c][s]
            assert exp.requests[c][s] == request
            if not exp.needs_action[c, s]:
                assert not exp.request_joint[c, s].any() and not exp.policy["ids"][c, s].any()
                continue
            direct = grid.encode_states([states[s]], [request], lines=[list(ref["logs"][c][s])])
            for key in search2.POLICY_KEYS:
                assert np.array_equal(exp.policy[key][c, s], direct[key][0]), (c, s, key)
            assert np.array_equal(exp.request_joint[c, s], P.request_mask(request)[1])
            assert np.array_equal(exp.engine_joint[c, s], exp.leaves[c].legal_mask(s)[1])
            # The leaf tracker was fed exactly once: encoding it with no lines gives the same row.
            again = grid.encode_states([exp.trackers[c][s]], [request])
            assert np.array_equal(again["ids"][0], direct["ids"][0])
            checked += 1
    assert checked >= len(actions)


def hidden_shadow_tag_battle():
    ours = [mon("Pikachu", ["fakeout", "protect", "thunderbolt", "volttackle"], "Lightning Rod"),
            mon("Raichu", ["protect", "thunderbolt", "nastyplot", "encore"], "Lightning Rod", "F")] + FILLER
    theirs = [mon("Gothitelle", ["fakeout", "protect", "psychic", "trickroom"], "Shadow Tag", "F"),
              mon("Indeedee", ["protect", "helpinghand", "shadowball", "encore"], "Psychic Surge", "F")] + FILLER
    battle = numbrion.Battle((1, 2, 3, 4), "]".join(ours), "]".join(theirs), names=("Alice", "Bob"), log=True)
    battle.start()
    return battle


def fresh_states(battle):
    return (numbrion.TrackerState(0).feed(battle.drain_player_log(0)),
            numbrion.TrackerState(1).feed(battle.drain_player_log(1)))


def test_leaf_masks_hide_an_unrevealed_shadow_tag_and_refusals_are_imputed(bot):
    battle = hidden_shadow_tag_battle()
    states = fresh_states(battle)
    protect = np.array([[14, 4], [14, 4]], dtype=np.int32)  # both sides: Protect, Protect (no reveal)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")  # a live battle as the root (a directed test)
        exp = expand(bot, [battle], protect[None], None, np.array([1, 2, 3, 4]), states, 0)
    assert not exp.ended[0] and tuple(exp.request_kind[0]) == (1, 1)
    request, engine = exp.request_joint[0, 0], exp.engine_joint[0, 0]
    assert json.loads(exp.requests[0][0])["active"][1].get("maybeTrapped")
    refused = request & ~engine
    assert refused.any() and not (engine & ~request).any()  # the request allows switches the engine refuses
    assert all(P.SWITCH_BASE <= a1 < P.PASS for _, a1 in np.argwhere(refused))
    # Layer 2's game at this leaf: a refused pair is worth our best simulated pair; a request-illegal one is invalid.
    leaf = _Leaf(key=(0, 0, 0, 0))
    search2._set_from(leaf, exp, 0, 0)
    leaf.their, leaf.their_prior = np.array([[14, 4], [24, 4]]), np.array([0.5, 0.5])
    ok_pairs = [tuple(int(v) for v in p) for p in np.argwhere(engine)[:2]]
    bad = tuple(int(v) for v in np.argwhere(refused)[0])
    illegal = next((a, b) for a in range(47) for b in range(47) if not request[a, b])
    leaf.rows = {ok_pairs[0]: np.array([0.1, 0.3]), ok_pairs[1]: np.array([0.5, 0.1])}
    run = object.__new__(_Deepen)
    run.side = 0
    a, valid = _Deepen.leaf_game(run, leaf, ok_pairs + [bad, illegal])
    assert valid.tolist() == [True, True, True, False]
    assert np.allclose(a[2], a[1])  # [0.5, 0.1] @ [0.5, 0.5] = 0.3 > 0.2


# ---- terminal leaves, ties and forced replacements ------------------------------------------------------------------

class _FakeExp:
    def __init__(self, ended, winner):
        n = len(ended)
        self.ended, self.winner = np.array(ended), np.array(winner, dtype=np.int8)
        self.needs_action = np.zeros((n, 2), dtype=bool)
        self.request_kind = np.zeros((n, 2), dtype=np.uint8)
        self.leaves = [None] * n
        self.trackers = [None] * n
        self.requests = [(None, None)] * n
        self.request_joint = self.engine_joint = np.zeros((n, 2, 47, 47), dtype=bool)


def synthetic_layer1(n_ours=3, n_worlds=2, m=3):
    rng = np.random.default_rng(1)
    payoffs = [rng.uniform(-0.5, 0.5, (n_ours, m)) for _ in range(n_worlds)]
    ys = [np.full(m, 1 / m) for _ in range(n_worlds)]
    x = np.array([0.6, 0.3, 0.1][:n_ours])
    q = np.mean([a @ y for a, y in zip(payoffs, ys)], axis=0)
    return Layer1(side=0, worlds=[None] * n_worlds, states=(None, None), ours=np.zeros((n_ours, 2), dtype=np.int32),
                  our_prior=np.full(n_ours, 1 / n_ours), their=[np.zeros((m, 2), dtype=np.int32)] * n_worlds,
                  their_prior=[np.full(m, 1 / m)] * n_worlds, legal=[np.ones(n_ours, bool)] * n_worlds,
                  payoffs=payoffs, x=x, ys=ys, q=q, seeds=[np.zeros((1, 4), dtype=np.int64)] * n_worlds)


def test_terminal_leaves_score_win_loss_tie_from_our_side_and_back_up_by_correction(bot):
    for side, winner, value in ((0, 0, 1.0), (0, 1, -1.0), (1, 1, 1.0), (1, 0, -1.0), (0, -1, 0.0), (1, -1, 0.0)):
        leaf = _Leaf(key=(0, 0, 0, 0))
        search2._set_from(leaf, _FakeExp([True], [winner]), 0, side)
        assert leaf.ended and leaf.value == value and leaf.battle is None
    l1 = synthetic_layer1()
    cfg2 = Depth2Config(enabled=True, theirs_top=2, danger=0)
    run = _Deepen(bot, bot.search, cfg2, l1, StageClock(), np.random.default_rng(0), "anchored", ("", ""), None)
    # Every deepened leaf of candidate 0 is a win, of candidate 1 a tie (ended leaves: not part of any group).
    for k in range(2):
        for i, v in ((0, 1.0), (1, 0.0)):
            for j, _ in run.reply_set(k, i):
                leaf = _Leaf(key=(k, i, j, 0), ended=True, terminal=v, value=v)
                run.leaves[(k, i, j, 0)] = leaf
    assert run.groups(list(run.leaves)) == {}
    Q2, score = run.backup([0, 1], [0, 1], 1)
    for i, v in ((0, 1.0), (1, 0.0)):
        expected = l1.q[i] + np.mean([sum(l1.ys[k][j] * (v - l1.payoffs[k][i][j]) for j, _ in run.reply_set(k, i))
                                      for k in range(2)])
        assert abs(Q2[i] - expected) < 1e-12
        assert abs(score[i] - (np.log(l1.our_prior[i]) + np.clip(Q2[i], -1, 1) / bot.search.anchor)) < 1e-9


def test_replies_are_the_top_by_weight_plus_the_most_dangerous(bot):
    l1 = synthetic_layer1(m=5)
    l1.ys[0] = np.array([0.5, 0.3, 0.15, 0.04, 0.01])
    l1.payoffs[0][0] = np.array([0.0, 0.0, 0.2, -0.4, -0.9])
    run = _Deepen(bot, bot.search, Depth2Config(enabled=True, theirs_top=2, danger=1, danger_min=0.02), l1,
                  StageClock(), np.random.default_rng(0), "anchored", ("", ""), None)
    assert run.reply_set(0, 0) == [(0, False), (1, False), (3, True)]  # reply 4 is below danger_min


def uturn_battle():
    ours = [mon("Scizor", ["uturn", "protect", "bulletpunch", "swordsdance"], "Technician"),
            mon("Amoonguss", ["spore", "protect", "ragepowder", "gigadrain"], "Regenerator", "F")] + FILLER
    theirs = [mon("Talonflame", ["uturn", "protect", "bravebird", "roost"], "Gale Wings"),
              mon("Indeedee", ["protect", "helpinghand", "shadowball", "encore"], "Psychic Surge", "F")] + FILLER
    battle = numbrion.Battle((1, 2, 3, 4), "]".join(ours), "]".join(theirs), log=True)
    battle.start()
    return battle


@pytest.mark.parametrize("steps", [0, 1, 2])
def test_forced_replacement_chains_are_stepped_to_a_move_turn(bot, steps):
    """Both sides U-turn: Talonflame's pivot asks side 1 for a switch, then Scizor's asks side 0. Each is answered by
    the policy argmax, up to ``replacement_steps``; then layer 2 starts at the next move turn."""
    battle = uturn_battle()
    states = fresh_states(battle)
    l1 = synthetic_layer1(n_ours=1, n_worlds=1, m=1)
    l1.worlds, l1.states = [battle], states
    l1.ours = np.array([[6, 14]], dtype=np.int32)  # U-turn -> foe a, Protect
    l1.their = [np.array([[6, 4]], dtype=np.int32)]  # U-turn -> foe a, Protect
    l1.seeds = [np.array([[1, 2, 3, 4]], dtype=np.int64)]
    l1.x, l1.q, l1.ys, l1.their_prior = np.ones(1), np.zeros(1), [np.ones(1)], [np.ones(1)]
    cfg2 = Depth2Config(enabled=True, replacement_steps=steps, ours=3, theirs=2, seeds=1)
    run = _Deepen(bot, bot.search, cfg2, l1, StageClock(), np.random.default_rng(0), "anchored", ("", ""), None)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        assert run.grow([(0, 0, 0, 0)])
        leaf = run.leaves[(0, 0, 0, 0)]
        assert leaf.steps == steps and run.forced_steps == steps
        assert leaf.kind == ((1, 1) if steps == 2 else (0, 2) if steps == 0 else (2, 0))
        assert search2._forced(leaf) == (steps < 2)
        if steps == 2:
            assert leaf.ours and len(leaf.their) >= 1 and leaf.seeds2 is not None
            # The leaf trackers saw both pivots and the switch-ins, fed once.
            assert leaf.trackers[0].turn == battle.turn + 1
        assert run.layer2([(0, 0, 0, 0)])
        assert leaf.value is not None and -1.0 <= leaf.value <= 1.0


# ---- budget integration, layer-1 chunks, threats --------------------------------------------------------------------

class CapClock(StageClock):
    """Admits chunks of at most ``cap`` units of the given stages, and refuses everything after ``limit`` calls."""

    def __init__(self, cap: dict, limit: int | None = None):
        super().__init__()
        self.cap, self.limit, self.calls = cap, limit, 0

    def ok(self, stage, units):
        self.calls += 1
        if self.limit is not None and self.calls > self.limit:
            self.refused = "limit"
            return False
        return units <= self.cap.get(stage, 10 ** 9)


def layer1_inputs(bot):
    from training.search import run_search

    pool = teams()
    battle = numbrion.Battle((2, 7, 1, 8), p1=pool[6], p2=pool[7], log=True)
    battle.start()
    player = bot.new_player("c", seed=4)
    player.feed(battle.drain_player_log(0))
    player.set_request(battle.request_json(0))
    request = json.loads(player.request_json)
    player._sync(request, 0)
    mask0, joint = P.request_mask(request)
    (pairs, _, full), = bot.net.candidates(player.battle.arrays(), np.array([0]), [(mask0, joint)], k=4, first=47,
                                           temperature=0.5)
    worlds = player.builder.worlds(4, 3, log=True)
    return run_search, player, worlds, pairs, full


def test_layer1_runs_in_admitted_chunks_of_whole_worlds(bot):
    run_search, player, worlds, pairs, full = layer1_inputs(bot)
    whole = run_search(bot, bot.search, player.battle, 0, worlds, pairs, full, rng=np.random.default_rng(0))
    per_world = len(pairs) * bot.search.theirs * bot.search.seeds
    clock = CapClock({"grid1": per_world, "value1": per_world})
    split = run_search(bot, bot.search, player.battle, 0, worlds, pairs, full, rng=np.random.default_rng(0),
                       clock=clock)
    # The same cells; the value net's float rounding may differ with the batch size (the solver amplifies it in x).
    assert len(split["worlds"]) == len(whole["worlds"]) == len(worlds)
    for a, b in zip(whole["payoffs"], split["payoffs"]):
        assert np.allclose(a, b, atol=1e-5)
    assert np.allclose(whole["x"], split["x"], atol=1e-2)
    grid_calls = [s for s in clock.stages if s.name == "grid1"]
    assert len(grid_calls) == len(worlds) and all(s.units <= per_world for s in grid_calls)
    # A clock that stops admitting after a few calls: the worlds already searched are kept, the rest dropped.
    clock = CapClock({"grid1": per_world, "value1": per_world}, limit=12)
    cut = run_search(bot, bot.search, player.battle, 0, worlds, pairs, full, rng=np.random.default_rng(0),
                     clock=clock)
    kept = len(cut["worlds"])
    assert 1 <= kept < len(worlds)
    l1 = cut["layer1"]
    assert len(l1.worlds) == len(l1.payoffs) == len(l1.ys) == len(l1.seeds) == len(l1.legal) == kept
    refuse_all = CapClock({}, limit=0)
    assert run_search(bot, bot.search, player.battle, 0, worlds, pairs, full, clock=refuse_all) == {"out_of_time": True}


def test_value_batches_are_gated_and_recorded(bot):
    from training.search import VALUE_BATCH, grid_values

    n = VALUE_BATCH + 5
    out = dict(ended=np.zeros(n, bool), winner=np.full(n, -1, np.int8), eval_side=np.zeros(n, np.int64))
    calls, record = [], []

    class Net:
        def values(self, arrays, partner_ids, partner_floats):
            calls.append(len(arrays["field"]))
            return np.full(len(arrays["field"]), 0.25, np.float32)

    for key in ("ids", "floats", "field", "token_features", "partner_ids", "partner_floats"):
        out[key] = np.zeros((n, 1), np.float32)
    v = grid_values(Net(), out, 0, gate=lambda stage, rows: True, record=lambda *a: record.append(a))
    assert calls == [VALUE_BATCH, 5] and [r[1] for r in record] == [VALUE_BATCH, 5] and np.allclose(v, 0.25)
    assert grid_values(Net(), out, 0, gate=lambda stage, rows: rows < 100) is None


def sleep_battle():
    ours = [mon("Pikachu", ["fakeout", "protect", "thunderbolt", "volttackle"], "Lightning Rod"),
            mon("Raichu", ["protect", "thunderbolt", "nastyplot", "encore"], "Lightning Rod", "F")] + FILLER
    theirs = [mon("Amoonguss", ["spore", "protect", "ragepowder", "gigadrain"], "Regenerator", "F"),
              mon("Indeedee", ["protect", "helpinghand", "shadowball", "yawn"], "Psychic Surge", "F")] + FILLER
    battle = numbrion.Battle((1, 2, 3, 4), "]".join(ours), "]".join(theirs), log=True)
    battle.start()
    return battle


def test_sleep_threats_are_flagged_and_deepened(bot):
    from training.search import layer1_difficulty

    battle = sleep_battle()
    joint = battle.legal_mask(1)[1]
    spore = next(tuple(int(v) for v in p) for p in np.argwhere(joint) if p[0] < 10 and 10 <= p[1] < 20)  # Spore, Protect
    plain = [tuple(int(v) for v in p) for p in np.argwhere(joint) if 20 <= p[0] < 40 and 20 <= p[1] < 30][:3]
    l1 = synthetic_layer1(n_ours=2, n_worlds=1, m=4)
    l1.worlds, l1.side = [battle], 0
    l1.their = [np.array(plain + [spore], dtype=np.int32)]
    l1.ys = [np.array([0.5, 0.3, 0.12, 0.08])]
    l1.payoffs = [np.array([[0.0, 0.1, -0.5, 0.2], [0.0, 0.1, -0.5, 0.2]])]
    request = json.loads(battle.request_json(0))
    hints = layer1_difficulty(l1, request)
    assert hints["known_threat"] and hints["threats"] == {"spore": 0.08} and hints["remaining"] == 6
    assert hints["ours_left"] == 6 and not hints["forced"]
    l1.ys = [np.array([0.5, 0.3, 0.19, 0.01])]
    assert not layer1_difficulty(l1, request)["known_threat"]
    l1.ys = [np.array([0.5, 0.3, 0.12, 0.08])]
    cfg2 = Depth2Config(enabled=True, theirs_top=2, danger=1)
    run = _Deepen(bot, bot.search, cfg2, l1, StageClock(), np.random.default_rng(0), "anchored", ("", ""), None,
                  threat_reply=True)
    assert run.reply_set(0, 0) == [(0, False), (1, False), (2, True), (3, True)]  # top 2, danger, sleep
    off = _Deepen(bot, bot.search, cfg2, l1, StageClock(), np.random.default_rng(0), "anchored", ("", ""), None)
    assert off.reply_set(0, 0) == [(0, False), (1, False), (2, True)]


@pytest.mark.parametrize("budget", [dict(mode="fixed"), dict(mode="adaptive", target_ms=1500, max_ms=4000)])
def test_the_budget_controller_plans_both_layers(checkpoint, budget):
    bot = SearchBot(checkpoint=str(checkpoint), search={"worlds": 3, "ours": 4, "theirs": 3, "seeds": 1, "threads": 2,
                                                        "iterations": 60, "max_worlds": 4, "depth2": SMALL},
                    pools=[str(POOL)], device="cpu", budget=budget)
    from training.search_budget import BudgetController

    assert isinstance(bot.budget, BudgetController)
    decisions, player = play(bot, seed=6, decisions=5,
                             hook=lambda p: setattr(p, "deadline", time.monotonic() + 30.0))
    assert decisions and not player.stats.get("depth2_errors")
    assert bot.budget.decisions == len([d for d in decisions if d.analysis.get("note") != "only one legal choice"])
    stats = bot.budget.stats()["stages"]
    assert stats["grid1"]["samples"] > 0 and stats["value1"]["samples"] > 0
    if budget["mode"] == "fixed":
        assert any(d.analysis.get("depth2", {}).get("rounds") == 2 for d in decisions)
        assert stats["grid2"]["samples"] > 0 and stats["expand"]["samples"] > 0


# ---- threat candidates (layer 1) and common opponent sets (layer 2) ---------------------------------------------------

def test_qualifying_threats_need_a_reveal_or_most_worlds():
    from training.search import qualifying_threats

    request = json.loads(sleep_battle().request_json(1))  # Amoonguss (Spore), Indeedee (Yawn)
    assert qualifying_threats([request, None], set(), 0.5) == {("amoonguss", "spore"), ("indeedee", "yawn")}
    assert qualifying_threats([request, None, None], set(), 0.5) == set()  # in 1 of 3 worlds: a phantom
    assert qualifying_threats([request, None, None], {("amoonguss", "spore")}, 0.5) == {("amoonguss", "spore")}


def test_threat_candidates_add_the_best_prior_sleep_pair_beyond_theirs():
    from training.search import add_threat_candidates

    battle = sleep_battle()
    request = json.loads(battle.request_json(1))
    legal = [tuple(int(v) for v in p) for p in np.argwhere(battle.legal_mask(1)[1])]
    rng = np.random.default_rng(0)
    full = {p: (float(rng.uniform(-9, -1)), float(rng.uniform(-9, -1))) for p in legal}
    spore = [p for p in legal if p[0] < 10]  # Amoonguss's move 1 is Spore
    yawn = [p for p in legal if 30 <= p[1] < 40]  # Indeedee's move 4 is Yawn
    for p in spore + yawn:
        full[p] = (full[p][0] - 20, full[p][1] - 20)  # never among the top pairs
    top = sorted((p for p in legal if p not in spore + yawn), key=lambda p: -full[p][0])[:3]
    pairs, logp = np.array(top, dtype=np.int32), np.array([full[p][1] for p in top], np.float32)
    out, out_logp, added = add_threat_candidates(request, pairs, logp, full, {("amoonguss", "spore")}, 2)
    assert added == 1 and len(out) == 4 and np.array_equal(out[:3], pairs)
    best = max(spore, key=lambda p: full[p][0])
    assert tuple(out[3]) == best and np.isclose(out_logp[3], full[best][1])  # the policy's own prior
    both = {("amoonguss", "spore"), ("indeedee", "yawn")}
    out, _, added = add_threat_candidates(request, pairs, logp, full, both, 2)
    assert added == 2 and any(30 <= p[1] < 40 for p in out)
    assert add_threat_candidates(request, pairs, logp, full, both, 1)[2] == 1  # at most `extra` per world
    with_spore = np.array(top[:2] + [spore[0]], dtype=np.int32)
    assert add_threat_candidates(request, with_spore, logp, full, {("amoonguss", "spore")}, 2)[2] == 0


def test_run_search_adds_threat_candidates_only_when_asked(bot):
    from dataclasses import replace

    from training.search import run_search

    battle = sleep_battle()
    player = bot.new_player("threat", seed=1)
    player.feed(battle.drain_player_log(0))
    battle.drain_player_log(1)
    player.set_request(battle.request_json(0))
    request = json.loads(player.request_json)
    player._sync(request, 0)
    mask0, joint = P.request_mask(request)
    (pairs, _, full), = bot.net.candidates(player.battle.arrays(), np.array([0]), [(mask0, joint)], k=4, first=47,
                                           temperature=0.5)
    revealed = {("amoonguss", "spore")}
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")  # the live battle as its own world (a directed test)
        off = run_search(bot, replace(bot.search, threat_candidates=False), player.battle, 0, [battle], pairs, full,
                         revealed=revealed)
        on = run_search(bot, replace(bot.search, threat_candidates=True), player.battle, 0, [battle], pairs, full,
                        revealed=revealed)
    assert off["threat_added"] == 0
    their = on["layer1"].their[0]
    assert on["threat_added"] >= 1 and len(their) == len(off["layer1"].their[0]) + on["threat_added"]
    assert any(p[0] < 10 for p in their)  # a Spore pair
    assert np.isclose(on["layer1"].their_prior[0].sum(), 1.0) and (on["layer1"].their_prior[0] > 0).all()
    assert on["layer1"].payoffs[0].shape[1] == len(their)
    assert replace(bot.search, threat_candidates=None).threats_on()  # on by default with depth2 enabled
    assert not replace(bot.search, threat_candidates=None, depth2=None).threats_on()


def test_common_opponent_sets_rank_by_mean_prior_cover_every_leaf_and_respect_masks():
    from training.search2 import common_opponent_sets

    a, b, c, d, e = (1, 46), (2, 46), (3, 46), (4, 46), (5, 46)
    mask_all = np.ones((47, 47), dtype=bool)
    mask_no_c = mask_all.copy()
    mask_no_c[c] = False  # e.g. the foe that would use it fainted in this leaf
    full1 = {a: (-0.5, -0.5), b: (-1.0, -1.0), c: (-2.0, -2.0), d: (-6.0, -6.0)}
    full2 = {a: (-0.7, -0.7), b: (-3.0, -3.0), e: (-0.2, -0.2), d: (-5.0, -5.0)}
    sets = common_opponent_sets([[a, b, c], [e, a, b]], [full1, full2], [mask_all, mask_no_c], 2)
    # Ranked by mean log-prior: a (-0.6), b (-2.0), then e and c below; size 2 keeps a, b; coverage adds e (leaf 2's
    # own top pair).
    pairs1, prior1 = sets[0]
    pairs2, prior2 = sets[1]
    assert [tuple(p) for p in pairs1] == [a, b]  # e is not in leaf 1's policy
    assert [tuple(p) for p in pairs2] == [a, b, e]
    assert np.isclose(prior1.sum(), 1) and np.isclose(prior2.sum(), 1)
    assert np.isclose(prior2[2] / prior2[0], np.exp(-0.2 + 0.7))  # each leaf's own prior, renormalised
    # A single leaf keeps exactly its own top pairs (the per-leaf behaviour).
    (alone, _), = common_opponent_sets([[a, b, c]], [full1], [mask_all], 3)
    assert [tuple(p) for p in alone] == [a, b, c]


def test_sibling_leaves_share_their_opponent_candidates(checkpoint, monkeypatch):
    captured = []
    original = _Deepen.run

    def run(self):
        info = original(self)
        captured.append(self)
        return info

    monkeypatch.setattr(_Deepen, "run", run)
    for common in (True, False):
        captured.clear()
        bot = make_bot(checkpoint)
        bot.search.depth2_common_opponent = common
        play(bot, seed=9, decisions=4)
        shared = differ = 0
        for dp in captured:
            groups: dict = {}
            for (k, i, j, c), lf in dp.leaves.items():
                if lf.ended or not lf.needs[dp.opp] or lf.their is None:
                    continue
                mask = _side_mask_key(lf, dp)
                groups.setdefault((k, j, c, mask), []).append(lf)
            for leaves in groups.values():
                for lf in leaves[1:]:
                    same = np.array_equal(lf.their, leaves[0].their)
                    shared += same
                    differ += not same
        if common:
            assert shared > 0 and differ == 0
        else:
            assert shared + differ > 0


def _side_mask_key(lf, dp):
    return search2._side_mask(lf, dp.opp, dp.side).tobytes()


def counter_layer1(n_replies=2):
    """Two candidates, equal priors. 0 is safe (0 against everything); 1 sets up: worth +0.5 a turn later if the
    opponent keeps doing what layer 1 expected (reply 0, y = 0.99), -0.8 if it counters (reply 1, layer-1 weight
    0.01, prior 0.3). At one ply the set-up looks slightly worse."""
    l1 = synthetic_layer1(n_ours=2, n_worlds=1, m=n_replies)
    l1.our_prior, l1.x = np.array([0.5, 0.5]), np.array([0.5, 0.5])
    row = [0.0, -0.2] + [0.0] * (n_replies - 2)
    l1.payoffs = [np.array([[0.0] * n_replies, row])]
    l1.ys = [np.array([0.99, 0.01] + [0.0] * (n_replies - 2))]
    l1.ys[0] = l1.ys[0] / l1.ys[0].sum()
    prior = np.array([0.7, 0.3] + [0.05] * (n_replies - 2))
    l1.their_prior = [prior / prior.sum()]
    l1.q = np.array([a @ l1.ys[0] for a in l1.payoffs[0]])
    return l1


def test_resolve_lets_the_opponent_counter_what_correction_assumes_it_will_not(bot):
    l1 = counter_layer1()

    def value(k, i, j):
        return 0.0 if i == 0 else (0.5 if j == 0 else -0.8)

    fixed = run_synthetic(bot, l1, value, theirs_top=2, danger=0, threat_reply=False)
    resolved = run_synthetic(bot, l1, value, theirs_top=2, danger=0, threat_reply=False, backup="resolve")
    assert fixed["chosen"] == 1 and fixed["rounds"][-1]["q2"][1] > 0.4  # correction: the opponent keeps y
    assert resolved["chosen"] == 0  # resolve: the opponent answers the set-up with the counter
    assert resolved["rounds"][-1]["q2"][1] < resolved["rounds"][-1]["q2"][0]


def test_resolve_keeps_undeepened_replies_at_their_layer1_values(bot):
    """Only reply 0 is deepened; the counter (reply 1) stays in the re-solved game at its layer-1 payoff (-0.9 for
    the set-up), so the opponent can still use it."""
    l1 = counter_layer1(n_replies=3)
    l1.payoffs = [np.array([[0.0, 0.0, 0.0], [0.0, -0.9, 0.0]])]
    l1.q = np.array([a @ l1.ys[0] for a in l1.payoffs[0]])

    def value(k, i, j):
        assert j == 0  # nothing else is deepened
        return 0.0 if i == 0 else 0.5

    resolved = run_synthetic(bot, l1, value, theirs_top=1, danger=0, threat_reply=False, backup="resolve")
    assert resolved["chosen"] == 0 and resolved["rounds"][-1]["q2"][1] < 0.0


def test_sleep_replies_below_danger_min_are_not_deepened(bot):
    battle = sleep_battle()
    joint = battle.legal_mask(1)[1]
    spore = next(tuple(int(v) for v in p) for p in np.argwhere(joint) if p[0] < 10 and 10 <= p[1] < 20)
    plain = [tuple(int(v) for v in p) for p in np.argwhere(joint) if 20 <= p[0] < 40 and 20 <= p[1] < 30][:3]
    l1 = synthetic_layer1(n_ours=2, n_worlds=1, m=4)
    l1.worlds, l1.side = [battle], 0
    l1.their = [np.array(plain + [spore], dtype=np.int32)]
    l1.payoffs = [np.array([[0.0, 0.1, -0.5, 0.2], [0.0, 0.1, -0.5, 0.2]])]
    cfg2 = Depth2Config(enabled=True, theirs_top=2, danger=1)

    def replies(ys, revealed=None):
        l1.ys = [np.array(ys)]
        run = _Deepen(bot, bot.search, cfg2, l1, StageClock(), np.random.default_rng(0), "anchored", ("", ""), None,
                      threat_reply=True, revealed=revealed)
        return run.reply_set(0, 0)

    # Spore far below danger_min: not deepened, even when revealed (it carries almost no weight in the backup).
    assert replies([0.6, 0.3, 0.0995, 0.0005]) == [(0, False), (1, False), (2, True)]
    assert replies([0.6, 0.3, 0.0995, 0.0005], revealed={("amoonguss", "spore")}) == [(0, False), (1, False), (2, True)]
    # Spore at y >= danger_min and outside the top/danger picks: deepened as the threat reply.
    assert replies([0.6, 0.3, 0.07, 0.03]) == [(0, False), (1, False), (2, True), (3, True)]
