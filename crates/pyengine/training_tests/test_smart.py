"""Tests for the doubles-aware scripted baseline (training.smart)."""
import sys
from copy import copy
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

import numbrion as nb
from training.dex import load_dex
from training.encoder import (FF, I_ABILITY, I_CAND_ABILITIES, I_ITEM, I_MOVE0, I_TERA, I_TYPE1, PF,
                              Buffers, Encoder)
from training.smart import FAKE_OUT_KO, PROTECT_MOVES, SmartHeuristicAgent

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"
PASS, N_ACTIONS = 46, 47


def status_position(move, slot=0):
    """Two healthy own actives, a paralyzed foe and an awake foe, with one selectable move."""
    torch.set_num_threads(1)
    dex = load_dex()
    arrays = Buffers(1).arrays()
    floats = arrays["floats"]
    for token in (0, 1, 6, 7):
        floats[0, token, PF["present"]] = floats[0, token, PF["hp"]] = 1
        floats[0, token, PF["active_a" if token in (0, 6) else "active_b"]] = 1
    floats[0, 6, PF["par"]] = 1
    floats[0, slot, PF["pp0"]] = 1
    arrays["ids"][0, slot, I_MOVE0] = dex.move_index[move]
    arrays["field"][0, FF["request_move"]] = 1
    return arrays


@pytest.mark.parametrize("slot", (0, 1))
def test_spore_never_targets_ally(slot):
    arrays = status_position("spore", slot)
    mask = np.zeros((1, N_ACTIONS), dtype=bool)
    ally_code = 2 * slot  # target code 0 = own b; code 1 = own a
    mask[0, [ally_code, 6, 8]] = True
    for seed in range(20):  # includes the original reproductions, seeds 1 and 3
        smart = SmartHeuristicAgent(np.random.default_rng(seed))
        if slot == 0:
            chosen = smart.first(arrays, np.array([0]), mask)
        else:
            mask0 = np.zeros_like(mask)
            mask0[0, PASS] = True
            smart.first(arrays, np.array([0]), mask0)
            chosen = smart.second(np.array([0]), mask)
        assert chosen[0] == 8, (slot, seed, chosen)  # the awake foe
        assert smart.ctx["e"]["act_bonus"][0, slot, ally_code] <= -1


@pytest.mark.parametrize("move", ("glare", "willowisp", "toxic", "taunt", "encore", "disable", "charm", "strengthsap"))
def test_harmful_status_penalizes_both_own_target_codes(move):
    smart = SmartHeuristicAgent(np.random.default_rng(0))
    effects = smart._prepare(status_position(move))["e"]
    assert (effects["act_bonus"][0, 0, :4] <= -1).all()


@pytest.mark.parametrize("move", ("healpulse", "floralhealing", "pollenpuff", "coaching", "helpinghand"))
def test_ally_support_keeps_its_value(move):
    arrays = status_position(move)
    arrays["floats"][0, 1, PF["hp"]] = 0.2
    smart = SmartHeuristicAgent(np.random.default_rng(0))
    effects = smart._prepare(arrays)["e"]
    assert effects["act_bonus"][0, 0, 0] >= 0
    if move in ("healpulse", "floralhealing", "pollenpuff"):
        assert effects["act_bonus"][0, 0, 0] > 0
        assert effects["act_bonus"][0, 0, 2] == 0  # full-HP own a
        assert effects["ally_d"][0, 0, 0] == effects["ally_k"][0, 0, 0] == 0
    if move == "helpinghand":
        assert effects["helping"][0, 0, 0] == 1


@pytest.mark.parametrize("airborne, probability", (
    ("flying", 1), ("tera_flying", 1), ("tera_grounded", 0), ("stellar_flying", 1),
    ("levitate", 1), ("believed_levitate", 0.35), ("known_overrides_belief", 0),
    ("balloon", 1), ("grounded", 0), ("gravity", 0),
))
def test_psychic_terrain_fake_out_checks_each_targets_grounding(airborne, probability):
    dex = load_dex()
    if airborne == "balloon" and "airballoon" not in dex.item_index:
        # The current exported random-doubles dex omits Balloon. Give it a valid item ID in a local dex.
        dex = copy(dex)
        dex.item_index = {**dex.item_index, "airballoon": dex.item_index["leftovers"]}
    arrays = status_position("fakeout")
    ids, floats, field = (arrays[k] for k in ("ids", "floats", "field"))
    ids[0, 6:8, I_ABILITY] = dex.ability_index["prankster"]
    ids[0, 6:8, I_TYPE1] = dex.type_idx("Normal")
    if airborne in ("flying", "tera_grounded", "stellar_flying", "gravity"):
        ids[0, 6, I_TYPE1 + 1] = dex.type_idx("Flying")
    if airborne in ("tera_flying", "tera_grounded", "stellar_flying"):
        floats[0, 6, PF["terastallized"]] = 1
        ids[0, 6, I_TERA] = dex.type_idx({"tera_flying": "Flying", "tera_grounded": "Water",
                                       "stellar_flying": "Stellar"}[airborne])
    if airborne == "levitate":
        ids[0, 6, I_ABILITY] = dex.ability_index["levitate"]
    if airborne in ("believed_levitate", "known_overrides_belief"):
        if airborne == "believed_levitate":
            ids[0, 6, I_ABILITY] = 0
        ids[0, 6, I_CAND_ABILITIES] = dex.ability_index["levitate"]
        floats[0, 6, PF["cand_ability_p0"]] = 0.35
        ids[0, 6, I_CAND_ABILITIES + 1] = dex.ability_index["prankster"]
        floats[0, 6, PF["cand_ability_p0"] + 1] = 0.65
    if airborne == "balloon":
        ids[0, 6, I_ITEM] = dex.item_index["airballoon"]
    if airborne == "gravity":
        field[0, FF["gravity"]] = 1
    smart = SmartHeuristicAgent(np.random.default_rng(0), dex)
    np.testing.assert_allclose(smart._can_flinch(ids, floats, field), [[1, 1]])
    field[0, FF["psychicterrain"]] = 1
    np.testing.assert_allclose(smart._can_flinch(ids, floats, field), [[probability, 0]])
    floats[0, 0, PF["fresh"]] = 1
    arrays["action_features"][0, 0, :, 6:8] = 1
    effects = smart._prepare(arrays)["e"]
    assert effects["flinch"][0, 0, 6, 0] == pytest.approx(probability)
    assert effects["flinch"][0, 0, 8, 1] == 0


def slot1_masks(env, needs, a0_smart, other_a0):
    """Slot-b masks of side 0 after smart's slot-a codes (side 1 keeps its own slot-a codes)."""
    joint = np.stack((np.where(needs, a0_smart, -1), other_a0), -1).astype(np.int32)
    mask1 = env.mask_slot1(joint)[:, 0].copy()
    mask1[~needs] = False
    mask1[~needs, PASS] = True
    return mask1


@pytest.fixture(scope="module")
def trajectory():
    """Smart plays side 0 against the engine's random player (switch- and Tera-happy, for varied positions).

    Returns every decision boundary (side 0's encoded arrays, masks and smart's codes) and side 0's player-view logs
    of the finished battles."""
    torch.set_num_threads(1)
    n = 16
    env = nb.BatchEnv(n, [str(POOL)], seed=21, threads=1, log=True)
    encoder = Encoder(2 * n)
    smart = SmartHeuristicAgent(np.random.default_rng(0))
    rows = 2 * np.arange(n)
    r = env.reset()
    steps, battles = [], []
    current = [[] for _ in range(n)]
    for _ in range(240):
        observations = [env.observe(e, s) for e in range(n) for s in (0, 1)]
        for e in range(n):
            own = observations[2 * e]
            if own["prev_log"]:
                battles.append(current[e] + list(own["prev_log"]))
                current[e] = []
            current[e].extend(own["log"])
        arrays = encoder.encode_batch(observations).copy()
        needs = r["needs_action"][:, 0].copy()
        mask0 = r["mask0"][:, 0].copy()
        mask0[~needs] = False
        mask0[~needs, PASS] = True
        actions = env.random_actions(switch_prob=0.15, tera_prob=0.2)
        a0 = smart.first(arrays, rows, mask0)
        mask1 = slot1_masks(env, needs, a0, actions[:, 1, 0])
        a1 = smart.second(rows, mask1)
        steps.append(dict(arrays={k: v[rows] for k, v in arrays.items()}, needs=needs, mask0=mask0, mask1=mask1,
                          a0=a0.copy(), a1=a1.copy()))
        actions[:, 0, 0] = np.where(needs, a0, -1)
        actions[:, 0, 1] = np.where(needs, a1, -1)
        r = env.step(actions)  # raises on any illegal action
    return steps, battles


def move_id(step, i, slot, code):
    return int(step["arrays"]["ids"][i, slot, I_MOVE0 + code // 10])


def test_actions_are_always_legal(trajectory):
    steps, battles = trajectory
    kinds = dict(moves=0, replacements=0, tera=0, switches=0)
    for step in steps:
        i = np.arange(len(step["a0"]))
        assert step["mask0"][i, step["a0"]].all()
        assert step["mask1"][i, step["a1"]].all()
        move_request = step["arrays"]["field"][:, FF["request_move"]] > 0.5
        acting = step["needs"]
        kinds["moves"] += int((acting & move_request).sum())
        kinds["replacements"] += int((acting & ~move_request).sum())
        codes = np.concatenate((step["a0"][acting], step["a1"][acting]))
        kinds["tera"] += int(((codes < 40) & (codes % 2 == 1)).sum())
        kinds["switches"] += int(((codes >= 40) & (codes < 46) & np.repeat(move_request[acting], 2)).sum())
    assert kinds["moves"] > 1500 and kinds["replacements"] > 50 and len(battles) > 30, kinds
    assert kinds["tera"] > 10 and kinds["switches"] > 0, kinds


def test_never_protects_twice_in_a_row(trajectory):
    steps, battles = trajectory
    dex = load_dex()
    protect_ids = {dex.move_index[m] for m in PROTECT_MOVES if m in dex.move_index}
    protects = blocked = 0
    for step in steps:
        floats = step["arrays"]["floats"]
        for slot, codes, mask in ((0, step["a0"], step["mask0"]), (1, step["a1"], step["mask1"])):
            for i, code in enumerate(codes):
                if not step["needs"][i] or code >= 40:
                    continue
                is_protect = move_id(step, i, slot, code) in protect_ids
                protects += is_protect
                if floats[i, slot, PF["protect_last"]] > 0.5:
                    legal = np.flatnonzero(mask[i])
                    only_protect = all(c < 40 and move_id(step, i, slot, c) in protect_ids for c in legal)
                    blocked += 1
                    assert not is_protect or only_protect
    assert protects > 20 and blocked > 10, (protects, blocked)
    # The same from the player-view logs: no own Pokemon uses a Protect move on two consecutive turns.
    names = {dex.moves[m - 1]["name"] for m in protect_ids}
    for lines in battles:
        turn, last = 0, {}
        for line in lines:
            parts = line.split("|")
            if len(parts) > 2 and parts[1] == "turn":
                turn = int(parts[2])
            elif len(parts) > 3 and parts[1] == "move" and parts[2].startswith("p1") and parts[3] in names:
                assert last.get(parts[2]) != turn - 1, f"repeated Protect on turn {turn}: {line}"
                last[parts[2]] = turn


def test_no_double_target_overkill(trajectory):
    """When one slot's attack KOs a foe for sure before either foe moves, and the partner has a real attack on the
    other foe, the partner does not also attack the doomed foe."""
    steps, _ = trajectory
    situations = violations = 0
    for step in steps:
        af = step["arrays"]["action_features"]
        floats = step["arrays"]["floats"]
        for i in np.flatnonzero(step["needs"]):
            codes = (step["a0"][i], step["a1"][i])
            for slot in (0, 1):
                code, other = codes[slot], codes[1 - slot]
                if code >= 40 or (code % 10) // 2 not in (3, 4):
                    continue
                target = (code % 10) // 2 - 3
                if af[i, slot, code, 2] < 0.95 or af[i, slot, code, 6:8].min() < 1:
                    continue  # not a sure KO before the foes act
                other_foe = 1 - target
                if floats[i, 6 + other_foe, PF["fainted"]] > 0.5 or floats[i, 6 + other_foe, PF["present"]] < 0.5:
                    continue
                mask = step["mask1"][i] if slot == 0 else step["mask0"][i]
                tc_other = 3 + other_foe
                alternatives = [c for c in np.flatnonzero(mask[:40]) if (c % 10) // 2 == tc_other
                                and af[i, 1 - slot, c, 0] >= 0.25]
                if not alternatives:
                    continue
                situations += 1
                same = other < 40 and (other % 10) // 2 == 3 + target and af[i, 1 - slot, other, 0] > 0
                violations += same
    assert situations >= 10, situations
    assert violations == 0, (violations, situations)


def test_fake_out_on_first_turn():
    """On turn 1 a lead with a legal Fake Out at a foe that is not immune uses it unless another attack likely KOs a
    foe, and never Fake Outs an immune foe."""
    torch.set_num_threads(1)
    dex = load_dex()
    fake_out = dex.move_index["fakeout"]
    smart = SmartHeuristicAgent(np.random.default_rng(1), dex)
    chances = used = at_immune = 0
    for seed in range(6):
        n = 64
        env = nb.BatchEnv(n, [str(POOL)], seed=300 + seed, threads=1, log=True)
        r = env.reset()
        encoder = Encoder(2 * n)
        arrays = encoder.encode_batch([env.observe(e, s) for e in range(n) for s in (0, 1)])
        rows = 2 * np.arange(n)
        assert (r["turn"] == 1).all()
        needs = r["needs_action"][:, 0].copy()
        mask0 = r["mask0"][:, 0].copy()
        a0 = smart.first(arrays.arrays(), rows, mask0)
        mask1 = slot1_masks(env, needs, a0, np.full(n, -1))
        a1 = smart.second(rows, mask1)
        ids, af, field = arrays.ids[rows], arrays.action_features[rows], arrays.field[rows]
        for i in range(n):
            if field[i, FF["psychicterrain"]] > 0.5:
                continue
            for slot, mask, chosen in ((0, mask0[i], a0[i]), (1, mask1[i], a1[i])):
                legal = np.flatnonzero(mask[:40])
                is_fake_out = ids[i, slot, I_MOVE0 + legal // 10] == fake_out
                options = [c for c in legal[is_fake_out] if (c % 10) // 2 in (3, 4)]
                sensible = [c for c in options if af[i, slot, c, 5] < 0.5]
                likely_ko = (af[i, slot, legal[~is_fake_out], 2] >= FAKE_OUT_KO).any()
                picked = chosen < 40 and ids[i, slot, I_MOVE0 + chosen // 10] == fake_out
                at_immune += picked and af[i, slot, chosen, 5] > 0.5
                if sensible and not likely_ko:
                    chances += 1
                    used += picked
    assert chances >= 15, chances
    assert at_immune == 0
    assert used >= 0.9 * chances, (used, chances)
