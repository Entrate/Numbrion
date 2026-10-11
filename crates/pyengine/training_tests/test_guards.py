"""Move-mask guards, protocol timing and agreement with the installed battle engine."""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

import numbrion as nb
from evaluate import PolicyAgent
from test_effects import INFERNAPE, SWITCH_IN, code, encode, request
from test_selfplay import POOL, request as team_request
from training.actors import Actors, buffer_layout
from training.dex import load_dex, to_id
from training.encoder import Buffers, Encoder
from training.guards import FIRST_TURN_ONLY, REPEAT_PROTECT, STALE, MoveGuards
from training.league import HEURISTIC, SELF, SNAPSHOT
from training.model import Model
from training.ppo import PASS
from training.vecenv import VecEnv


def reference_mask(mask, guard, slot, guards):
    """Deliberately scalar oracle, including rows with zero legal moves."""
    expected = mask.copy()
    changed = np.zeros(len(mask), dtype=bool)
    for row in range(len(mask)):
        blocked = {m for m in range(4) if (guards.stale and guard[row, slot, m, STALE])
                   or (guards.repeat_protect and guard[row, slot, m, REPEAT_PROTECT])}
        if any(mask[row, c] and c // 10 not in blocked for c in range(40)):
            for c in range(40):
                if c // 10 in blocked:
                    expected[row, c] = False
            changed[row] = not np.array_equal(expected[row], mask[row])
    return expected, changed


@pytest.mark.parametrize("slot", [0, 1])
@pytest.mark.parametrize("stale,repeat", [(True, True), (True, False), (False, True), (False, False)])
def test_blocked_covers_every_target_and_tera_code_for_the_selected_slot(slot, stale, repeat):
    guard = np.zeros((2, 2, 4, 2), dtype=bool)
    guard[0, 0, 0, STALE] = guard[0, 1, 3, STALE] = True
    guard[0, 0, 2, REPEAT_PROTECT] = guard[0, 1, 1, REPEAT_PROTECT] = True
    guard[1, 1 - slot] = True  # Opposite active's flags must not leak into this slot.
    before = guard.copy()
    blocked = MoveGuards(stale, repeat).blocked(guard, slot)
    expected = np.zeros((2, 47), dtype=bool)
    if stale:
        expected[0, (0 if slot == 0 else 30):(10 if slot == 0 else 40)] = True
    if repeat:
        expected[0, (20 if slot == 0 else 10):(30 if slot == 0 else 20)] = True
    np.testing.assert_array_equal(blocked, expected)
    np.testing.assert_array_equal(guard, before)
    assert blocked.dtype == bool and not blocked[:, 40:].any()


def test_apply_preserves_only_moves_switches_pass_and_reports_changed_rows():
    guard = np.zeros((7, 2, 4, 2), dtype=bool)
    guard[:, 0, 0, STALE] = True
    guard[3, 0, :, STALE] = True
    mask = np.zeros((7, 47), dtype=bool)
    mask[0, [6, 7, 16, 17, 42]] = True  # Remove first move; preserve second and switch.
    mask[1, [6, 7, 42]] = True  # Only-move exception even with a legal switch.
    mask[2, PASS] = True
    mask[3, :40] = True  # All four moves guarded: retain all of them.
    mask[4, [16, 17, 42]] = True  # Flags exist, but no flagged code was legal.
    mask[5, [40, 41, PASS]] = True  # Forced switch, with optional PASS.
    before, guard_before = mask.copy(), guard.copy()
    actual, changed = MoveGuards().apply(mask, guard, 0)
    expected = before.copy()
    expected[0, [6, 7]] = False
    np.testing.assert_array_equal(actual, expected)
    np.testing.assert_array_equal(changed, [True, False, False, False, False, False, False])
    np.testing.assert_array_equal(mask, before)
    np.testing.assert_array_equal(guard, guard_before)
    assert not np.shares_memory(actual, mask)
    disabled, changed = MoveGuards(False, False).apply(mask, guard, 0)
    np.testing.assert_array_equal(disabled, mask)
    assert not changed.any() and not np.shares_memory(disabled, mask)


def test_apply_matches_scalar_oracle_on_random_and_empty_batches():
    rng = np.random.default_rng(14)
    mask = (rng.random((600, 47)) < 0.15)[::2]  # Noncontiguous inputs.
    guard = (rng.random((600, 2, 4, 2)) < 0.3)[::2]
    for slot in (0, 1):
        for guards in (MoveGuards(), MoveGuards(False, True), MoveGuards(True, False), MoveGuards(False, False)):
            actual, changed = guards.apply(mask, guard, slot)
            expected, expected_changed = reference_mask(mask, guard, slot, guards)
            np.testing.assert_array_equal(actual, expected)
            np.testing.assert_array_equal(changed, expected_changed)
    empty, changed = MoveGuards().apply(mask[:0], guard[:0], 0)
    assert empty.shape == (0, 47) and changed.shape == (0,)


@pytest.mark.parametrize("move", FIRST_TURN_ONLY)
@pytest.mark.parametrize("action", ["|move|p1a: Infernape|Close Combat|p2a: Inteleon",
                                   "|cant|p1a: Infernape|flinch"])
def test_encoder_first_turn_moves_are_fresh_then_stale_after_move_or_flinch(move, action):
    moves = [move, "closecombat", "protect", "flareblitz"]
    fresh = encode([SWITCH_IN + ["|turn|1"]], moves=moves)
    assert not fresh.guard.any()
    stale = encode([SWITCH_IN + ["|turn|1", action, "|turn|2"]], moves=moves)
    expected = np.zeros((2, 2, 4, 2), dtype=bool)
    expected[0, 0, 0, STALE] = True
    np.testing.assert_array_equal(stale.guard, expected)


def test_encoder_switch_out_and_back_resets_both_guard_kinds():
    encoder = Encoder(2)
    first = SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Protect|p1a: Infernape",
                         "|-singleturn|p1a: Infernape|Protect", "|turn|2"]
    encoder.encode(0, dict(battle_id=1, log=first, request=request(), turn=2))
    assert encoder.buffers.guard[0, 0, 0, STALE] and encoder.buffers.guard[0, 0, 2, REPEAT_PROTECT]
    encoder.encode(0, dict(battle_id=1, log=["|switch|p1a: Pikachu|Pikachu, L85|200/200", "|turn|3",
                                          "|switch|p1a: Infernape|Infernape, L85|200/250", "|turn|4"],
                           request=request(), turn=4))
    assert not encoder.buffers.guard.any()


@pytest.mark.parametrize("protect", ["Protect", "Burning Bulwark", "Spiky Shield", "Baneful Bunker"])
def test_encoder_repeat_protect_success_failure_rest_and_cross_move_counter(protect):
    moves = ["fakeout", to_id(protect), "protect", "closecombat"]
    turn1 = SWITCH_IN + ["|turn|1", f"|move|p1a: Infernape|{protect}|p1a: Infernape",
                         f"|-singleturn|p1a: Infernape|{protect}", "|turn|2"]
    b = encode([turn1], moves=moves)
    np.testing.assert_array_equal(b.guard[0, 0, :, REPEAT_PROTECT], [False, True, True, False])
    failed = [f"|move|p1a: Infernape|{protect}||[still]", "|-fail|p1a: Infernape", "|turn|3"]
    assert not encode([turn1, failed], moves=moves).guard[..., REPEAT_PROTECT].any()
    rested = ["|move|p1a: Infernape|Close Combat|p2a: Inteleon", "|turn|3"]
    assert not encode([turn1, rested], moves=moves).guard[..., REPEAT_PROTECT].any()
    first_failed = SWITCH_IN + ["|turn|1", f"|move|p1a: Infernape|{protect}||[still]",
                                "|-fail|p1a: Infernape", "|turn|2"]
    assert not encode([first_failed], moves=moves).guard[..., REPEAT_PROTECT].any()


def test_encoder_guards_use_short_request_move_list_and_slot_one():
    req = json.loads(team_request())
    req["active"][0]["moves"] = [dict(id="fakeout", pp=2, maxpp=10, disabled=False)]
    req["active"][1]["moves"] = [dict(id="firstimpression", pp=2, maxpp=10, disabled=False)]
    lines = ["|switch|p1a: Dipplin|Dipplin, L90|100/200", "|switch|p1b: Volbeat|Volbeat, L90|100/200",
             "|turn|1", "|move|p1a: Dipplin|Protect|p1a: Dipplin", "|-singleturn|p1a: Dipplin|Protect",
             "|cant|p1b: Volbeat|flinch", "|turn|2"]
    encoder = Encoder(2)
    encoder.encode(0, dict(battle_id=1, log=lines, request=json.dumps(req), turn=2))
    expected = np.zeros((2, 2, 4, 2), dtype=bool)
    expected[0, :, 0, STALE] = True  # Team list has Protect first, but the locked request indexes Fake Out first.
    np.testing.assert_array_equal(encoder.buffers.guard, expected)
    assert "guard" not in Buffers.KEYS and "guard" not in buffer_layout(1, 2)


@pytest.mark.parametrize("kind", ["wait", "forced", "no_active", "inactive", "fainted", "empty", "new_battle"])
def test_encoder_clears_guard_on_non_move_requests_and_new_battles(kind):
    encoder = Encoder(2)
    lines = SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Protect|p1a: Infernape",
                        "|-singleturn|p1a: Infernape|Protect", "|turn|2"]
    encoder.encode(0, dict(battle_id=1, log=lines, request=request(), turn=2))
    assert encoder.buffers.guard.any()
    req = json.loads(request())
    if kind == "wait":
        req["wait"] = True
    elif kind == "forced":
        req["forceSwitch"] = [True, False]
    elif kind == "no_active":
        req.pop("active")
    elif kind == "inactive":
        req["side"]["pokemon"][0]["active"] = False
    elif kind == "fainted":
        req["side"]["pokemon"][0]["condition"] = "0 fnt"
    encoder.encode(0, dict(battle_id=2 if kind == "new_battle" else 1, log=[],
                           request="" if kind == "empty" else json.dumps(req), turn=2))
    assert not encoder.buffers.guard.any()


@pytest.mark.parametrize("prior_record", [False, True])
def test_encoder_never_flags_our_disguised_zoroark_even_with_a_stale_old_record(prior_record):
    req = json.loads(request(moves=["fakeout", "firstimpression", "protect", "spikyshield"], ability="illusion"))
    req["side"]["pokemon"][0].update(ident="p1: Zoroark", details="Zoroark, L85")
    previous = (["|switch|p1a: Zoroark|Zoroark, L85|250/250", "|turn|1",
                 "|move|p1a: Zoroark|Protect|p1a: Zoroark", "|-singleturn|p1a: Zoroark|Protect"]
                if prior_record else [])
    lines = previous + SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Protect|p1a: Infernape",
                                   "|-singleturn|p1a: Infernape|Protect", "|turn|2"]
    encoder = Encoder(2)
    encoder.encode(0, dict(battle_id=1, log=lines, request=json.dumps(req), turn=2))
    assert encoder.trackers[0].active["p1a"] == ("p1", "Infernape")
    assert not encoder.buffers.guard.any()


def test_guards_agree_with_executed_first_turn_moves_in_many_random_battles():
    env = nb.BatchEnv(48, [str(POOL)], seed=19, threads=2, log=True)
    env.reset()
    encoder = Encoder(96)
    counts, mismatches, fresh_failures = Counter(), [], []
    observations = [env.observe(e, s) for e in range(48) for s in (0, 1)]
    for boundary in range(320):
        b = encoder.encode_batch(observations)
        actions = env.random_actions(switch_prob=0.08, tera_prob=0.1)
        selected = []
        for row, observation in enumerate(observations):
            req = json.loads(observation["request"]) if observation["request"] else {}
            tracker = encoder.trackers[row]
            team = req.get("side", {}).get("pokemon", [])
            for slot, active in enumerate(req.get("active") or []):
                c = int(actions[row // 2, row % 2, slot])
                moves = (active or {}).get("moves", [])
                if not 0 <= c < 40 or c // 10 >= len(moves) or slot >= len(team):
                    continue
                move = moves[c // 10]["id"]
                name = team[slot]["ident"].split(": ", 1)[1]
                if move not in FIRST_TURN_ONLY:
                    continue
                known = tracker.active.get(tracker.own + "ab"[slot]) == (tracker.own, name)
                if not known:
                    counts["unknown_selected"] += 1
                    continue
                selected.append((row, slot, name, move, bool(b.guard[row, slot, c // 10, STALE])))
        result = env.step(actions)
        assert not result["illegal"].any()
        counts["completed_battles"] += int(result["done"].sum())
        observations = [env.observe(e, s) for e in range(48) for s in (0, 1)]
        for row, slot, name, move, stale in selected:
            obs = observations[row]
            lines = obs["prev_log"] if result["done"][row // 2] else obs["log"]
            ident = f"p{row % 2 + 1}{'ab'[slot]}: {name}"
            indices = [i for i, line in enumerate(lines) if line.startswith(f"|move|{ident}|")
                       and to_id(line.split("|")[3]) == move and "[from]" not in line]
            if not indices:  # Fainted or |cant| before it could execute.
                counts["not_executed"] += 1
                continue
            i = indices[0]
            end = next((j for j in range(i + 1, len(lines)) if lines[j].startswith(("|move|", "|turn|", "|upkeep|"))), len(lines))
            follow = lines[i + 1:end]
            failed = any(line.startswith(f"|-fail|{ident}") for line in follow)
            priority_block = any(line.startswith("|cant|") and f"[of] {ident}" in line for line in follow)
            failed |= priority_block  # Armor Tail reports |cant| on its holder, naming the attacker in [of].
            counts["priority_block_before_staleness_check"] += int(stale and priority_block)
            stale_hint = any("only works on your first turn out" in line for line in follow)
            counts[f"{'stale' if stale else 'fresh'}_{move}_executed"] += 1
            if stale and not failed or not stale and stale_hint:
                intermediate = [line for line in lines[:i] if line.startswith(f"|move|{ident}|")]
                mismatch = dict(boundary=boundary, row=row, slot=slot, move=move, stale=stale,
                                failed=failed, intermediate=intermediate, log=lines)
                mismatches.append(mismatch)
            if not stale and failed:
                counts["fresh_generic_fail"] += 1
                fresh_failures.append(dict(boundary=boundary, row=row, move=move, log=lines))
            counts["stale_hint"] += int(stale_hint)
        # Attribute extra executions that can alter the tracker independently of selected actions.
        for e in range(48):
            lines = observations[2 * e]["prev_log"] if result["done"][e] else observations[2 * e]["log"]
            for line in lines:
                if line.startswith("|move|") and "[from] ability:" in line:
                    counts[line.split("[from] ability: ", 1)[1].split("|", 1)[0]] += 1
    assert counts["completed_battles"] >= 100
    for move in FIRST_TURN_ONLY:
        assert counts[f"stale_{move}_executed"] > 10 and counts[f"fresh_{move}_executed"] > 10
    assert not mismatches, json.dumps(mismatches[:2], indent=2)


def test_magic_bounce_reflection_does_not_make_the_next_fake_out_stale():
    """A bounced move is not a move action (the engine's useMove), so Fake Out still works the next turn. The custom
    team gives Infernape Magic Bounce to isolate the case."""
    def packed(name, ability, moves):
        return f"{name}||Leftovers|{ability}|{moves}||85,85,85,85,85,85||||90|,,,,,Water"

    own = "]".join([packed("Pikachu", "Static", "protect,thunderbolt"),
                    packed("Tropius", "Harvest", "protect,energyball"),
                    packed("Infernape", "MagicBounce", "fakeout,protect,closecombat,flareblitz")])
    foe = "]".join([packed("Raichu", "Static", "thunderwave,protect"),
                    packed("Inteleon", "Torrent", "protect,scald")])
    battle = nb.Battle((1, 2, 3, 4), own, foe, log=True)
    battle.start()
    encoder = Encoder(2)

    def observe():
        lines = battle.drain_player_log(0)
        encoder.encode(0, dict(battle_id=1, log=lines, request=battle.request_json(0), turn=battle.turn))
        return lines

    observe()
    battle.choose(0, "switch 3, move 1")
    battle.choose(1, "move 1 1, move 1")
    reflected = observe()
    assert any("|move|p1a: Infernape|Thunder Wave|" in line and "Magic Bounce" in line for line in reflected)
    flagged = bool(encoder.buffers.guard[0, 0, 0, STALE])
    raw, _ = battle.legal_mask(0)
    guarded, changed = MoveGuards().apply(raw[None], encoder.buffers.guard[:1], 0)
    removed_fake_out = bool(raw[code(0, 3)] and not guarded[0, code(0, 3)])
    battle.choose(0, "move 1 1, move 1")
    battle.choose(1, "move 1 1, move 1")
    executed = observe()
    assert "|move|p1a: Infernape|Fake Out|p2a: Raichu" in executed
    assert any(line.startswith("|-damage|p2a: Raichu|") for line in executed)
    assert "|cant|p2a: Raichu|flinch" in executed
    assert not any("Fake Out only works" in line for line in executed)
    assert not flagged, f"Magic Bounce falsely marked Fake Out stale; removed={removed_fake_out}, changed={changed.tolist()}; engine Fake Out dealt damage and flinched"


@pytest.mark.parametrize("guards", [MoveGuards(), MoveGuards(repeat_protect=False)])
def test_real_actors_store_guarded_policy_masks_and_raw_scripted_masks(guards):
    torch.manual_seed(5)
    model = Model(load_dex(), width=32, layers=1)
    opponents = np.array([SELF, SNAPSHOT, HEURISTIC, HEURISTIC] * 2, dtype=np.int8)
    with Actors(8, 96, [POOL], seed=13, workers=2, model=model, guards=guards) as actors:
        actors.set_weights("learner", model.state_dict())
        actors.set_weights("opponent", model.state_dict())
        actors.start(0, opponents, ko_bonus=0.0)
        totals = actors.wait()
        data = {k: v.copy() for k, v in actors.buffers[0].arrays.items()}
    guarded_count = scripted_flags = snapshot_flags = 0
    for worker in range(2):
        env = nb.BatchEnv(4, [str(POOL)], seed=13000 + worker, threads=1, log=True)
        result = env.reset()
        encoder = Encoder(8)
        rows = slice(worker * 8, (worker + 1) * 8)
        scripted = np.array([False, False, False, False, False, True, False, True])
        snapshot = np.array([False, False, False, True, False, False, False, False])
        for t in range(96):
            b = encoder.encode_batch([env.observe(e, s) for e in range(4) for s in (0, 1)])
            active = result["needs_action"].reshape(-1)
            np.testing.assert_array_equal(data["ids"][t, rows], b.ids)
            a0, a1 = data["a0"][t, rows], data["a1"][t, rows]
            raw0 = result["mask0"].reshape(-1, 47).copy()
            raw1 = env.mask_slot1(np.where(active, a0, -1).reshape(4, 2).astype(np.int32)).reshape(-1, 47).copy()
            for slot, raw in enumerate((raw0, raw1)):
                raw[~active] = False
                raw[~active, PASS] = True
                expected, changed = reference_mask(raw, b.guard, slot, guards)
                scripted_flags += int((changed & scripted).sum())
                snapshot_flags += int((changed & snapshot).sum())
                expected[scripted] = raw[scripted]
                stored = data[f"mask{slot}"][t, rows]
                np.testing.assert_array_equal(stored, expected, err_msg=f"worker={worker}, t={t}, slot={slot}")
                picks = a0 if slot == 0 else a1
                assert stored[np.arange(8), picks].all()
                guarded_count += int((changed & data["train"][t, rows]).sum())
            actions = np.stack((a0, a1), -1).reshape(4, 2, 2).astype(np.int32)
            actions[~result["needs_action"]] = -1
            result = env.step(actions)
            assert not result["illegal"].any()
    assert guarded_count == totals["guarded"] and guarded_count > 0
    assert scripted_flags > 0
    if guards.repeat_protect:
        assert snapshot_flags > 0


@pytest.mark.parametrize("guards,include_guard", [(None, True), (MoveGuards(), True), (MoveGuards(), False)])
def test_policy_agent_uses_both_guarded_masks_through_real_vecenv(guards, include_guard, monkeypatch):
    torch.set_num_threads(1)
    torch.manual_seed(9)
    model = Model(load_dex(), width=32, layers=1).eval()
    seen = {}
    original0, original1 = model.slot0, model.slot1

    def slot0(state, mask):
        seen[0] = mask.numpy().copy()
        return original0(state, mask)

    def slot1(state, vectors, a0, mask):
        seen[1] = mask.numpy().copy()
        return original1(state, vectors, a0, mask)

    monkeypatch.setattr(model, "slot0", slot0)
    monkeypatch.setattr(model, "slot1", slot1)
    agent = PolicyAgent(model, "cpu", np.random.default_rng(9), "test", guards=guards)
    changed_count = 0
    with VecEnv(4, [POOL], seed=23, workers=2) as env:
        arrays = env.reset()
        for _ in range(64):
            active = arrays["needs_action"].reshape(-1).copy()
            rows = np.flatnonzero(active)[::-1]  # Check guard row selection and retained ordering.
            raw0 = arrays["mask0"].reshape(-1, 47)[rows].copy()
            guard = arrays["guard"][rows].copy()
            view = arrays if include_guard else {k: v for k, v in arrays.items() if k != "guard"}
            a0 = agent.first(view, rows, raw0)
            first = np.full(8, -1, dtype=np.int32)
            first[rows] = a0
            raw1 = env.mask_slot1(first.reshape(4, 2)).reshape(-1, 47)[rows].copy()
            # VecEnv owns shared arrays: second() must use the observation copied by first().
            arrays["guard"][rows] = False
            a1 = agent.second(rows, raw1)
            for slot, raw in enumerate((raw0, raw1)):
                expected, changed = reference_mask(raw, guard, slot, guards or MoveGuards(False, False))
                if not include_guard:
                    expected = raw
                else:
                    changed_count += int(changed.sum())
                np.testing.assert_array_equal(seen[slot], expected)
                assert raw[np.arange(len(rows)), a0 if slot == 0 else a1].all()
            actions = np.full((8, 2), -1, dtype=np.int32)
            actions[rows] = np.stack((a0, a1), -1)
            arrays = env.step(actions.reshape(4, 2, 2))  # VecEnv raises on any illegal pair.
    if guards is not None and include_guard:
        assert changed_count > 0
