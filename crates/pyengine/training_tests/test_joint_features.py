"""Slot b's partner-aware joint features (training.model.JOINT_COLUMNS, ``--joint-features``)."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

nb = pytest.importorskip("numbrion")
if not hasattr(nb, "ObsEncoder"):
    pytest.skip("wheel lacks the Rust encoder", allow_module_level=True)

from training.damage import ACTION_COLUMNS
from training.dex import load_dex
from training.encoder import (I_MOVE0, LEGACY_ACTION_FEATURES, N_ACTION_FEATURES, N_FIELD_FLOATS, N_IDS,
                              N_POKEMON_FLOATS, N_TOKEN_FEATURES, PF, RustEncoder)
from training.model import JOINT_COLUMNS, Model, ko_reach

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"
A = {name: LEGACY_ACTION_FEATURES + i for i, name in enumerate(ACTION_COLUMNS)}
J = {name: i for i, name in enumerate(JOINT_COLUMNS)}
KEYS = ("ids", "floats", "field", "action_features", "token_features")


def code(move, target=3, tera=False):
    return move * 10 + target * 2 + int(tera)


def observation():
    """Two rows from the Rust encoder: one side with real damage features (slot a knows Helping Hand), one empty."""
    team = [("Infernape", ["thunderbolt", "helpinghand", "flamethrower", "heatwave"]),
            ("Dipplin", ["energyball", "dracometeor", "protect"]), ("Jolteon", ["thunderbolt"]),
            ("Flareon", ["flamethrower"])]
    pokemon = [{"ident": f"p1: {name}", "details": f"{name}, L80", "condition": "250/250", "active": i < 2,
                "stats": {"atk": 150, "spa": 150, "def": 150, "spd": 150, "spe": 150}, "moves": known, "item": "",
                "ability": "blaze", "teraType": "Fire"} for i, (name, known) in enumerate(team)]
    active = [{"moves": [{"id": m, "pp": 8, "maxpp": 8, "disabled": False} for m in known], "canTerastallize": "Fire"}
              for _, known in team[:2]]
    req = json.dumps({"active": active, "side": {"id": "p1", "pokemon": pokemon}})
    lines = ["|switch|p1a: Infernape|Infernape, L80|250/250", "|switch|p1b: Dipplin|Dipplin, L80|250/250",
             "|switch|p2a: Inteleon|Inteleon, L80|100/100", "|switch|p2b: Ditto|Ditto, L80|100/100", "|turn|1",
             "|-damage|p2a: Inteleon|30/100", "|turn|2"]
    enc = RustEncoder(2)
    enc.encode_batch([{"battle_id": 1, "log": lines, "request": req, "turn": 2},
                      {"battle_id": 1, "log": [], "request": "", "turn": 2}])
    return {k: torch.from_numpy(getattr(enc.buffers, k).copy()) for k in KEYS}


def policy(model, t, a0, mask=None):
    mask = torch.ones(len(a0), 47, dtype=torch.bool) if mask is None else mask
    state = model.trunk(t["ids"], t["floats"], t["field"], t["action_features"], t["token_features"])
    lp0, vectors0 = model.slot0(state, mask)
    return lp0, model.slot1(state, vectors0, a0, mask)


def randomize_joint(model, seed=1):
    generator = torch.Generator().manual_seed(seed)
    with torch.no_grad():
        model.act_joint.weight.copy_(torch.randn(model.act_joint.weight.shape, generator=generator))
        model.act_joint.bias.copy_(torch.randn(model.act_joint.bias.shape, generator=generator) * 0.1)


def test_flag_off_model_is_unchanged():
    dex = load_dex()
    torch.manual_seed(0)
    old = Model(dex, width=32, layers=1)
    torch.manual_seed(0)
    off = Model(dex, width=32, layers=1, joint_features=False)
    torch.manual_seed(0)
    on = Model(dex, width=32, layers=1, joint_features=True)
    assert not off.joint_features and not hasattr(off, "act_joint")
    assert list(off.state_dict()) == list(old.state_dict())
    for key, value in old.state_dict().items():
        torch.testing.assert_close(off.state_dict()[key], value, rtol=0, atol=0)
        torch.testing.assert_close(on.state_dict()[key], value, rtol=0, atol=0)  # same random draws
    assert set(on.state_dict()) - set(old.state_dict()) == {"act_joint.weight", "act_joint.bias"}
    t = observation()
    a0 = torch.tensor([code(1, 0), 46])
    for x, y in zip(policy(old, t, a0), policy(off, t, a0)):
        torch.testing.assert_close(x, y, rtol=0, atol=0)


def test_zero_joint_layer_keeps_the_policy_and_trained_one_moves_only_slot_b():
    dex = load_dex()
    torch.manual_seed(0)
    off = Model(dex, width=32, layers=1)
    on = Model(dex, width=32, layers=1, joint_features=True)
    assert not on.act_joint.weight.any() and not on.act_joint.bias.any()
    missing, unexpected = on.load_state_dict(off.state_dict(), strict=False)
    assert set(missing) == {"act_joint.weight", "act_joint.bias"} and not unexpected
    t = observation()
    for a0 in ([code(0, 3), 46], [code(1, 0), 40], [code(3, 2, True), 41]):
        a0 = torch.tensor(a0)
        lp0_off, lp1_off = policy(off, t, a0)
        lp0_on, lp1_on = policy(on, t, a0)
        torch.testing.assert_close(lp0_on, lp0_off)
        torch.testing.assert_close(lp1_on, lp1_off)
    randomize_joint(on)
    a0 = torch.tensor([code(0, 3), 46])
    lp0_off, lp1_off = policy(off, t, a0)
    lp0_on, lp1_on = policy(on, t, a0)
    torch.testing.assert_close(lp0_on, lp0_off)  # slot a never sees the joint features
    assert not torch.allclose(lp1_on[0, :40], lp1_off[0, :40])


def test_features_from_the_rust_encoder():
    dex = load_dex()
    model = Model(dex, width=32, layers=1, joint_features=True)
    t = observation()
    af = t["action_features"]
    assert af[0, 0, code(0, 3), A["foe_a_damage"]] > 0  # Thunderbolt on Inteleon
    state = model.trunk(t["ids"], t["floats"], t["field"], af, t["token_features"])
    torch.testing.assert_close(state["foe_hp"][0], torch.tensor([0.3, 1.0]))
    onehot = torch.nn.functional.one_hot(torch.tensor([code(0, 3), code(0, 3)]), 47).float()
    x = model.partner_features(state, onehot)
    assert x.shape == (2, 40, len(JOINT_COLUMNS)) and torch.isfinite(x).all()
    assert x[0, 0, J["left_a"]] == pytest.approx(max(0.3 - float(af[0, 0, code(0, 3), A["foe_a_damage"]]), 0), abs=1e-6)
    assert not x[1].any()  # the empty row has no foes
    hh = model.partner_features(state, torch.nn.functional.one_hot(torch.tensor([code(1, 0, True), 46]), 47).float())
    assert (hh[0, :, J["helping_hand"]] == 1).all() and not hh[1].any()


def hand_built(model):
    """Rows: 0 slot a surely KOs foe a; 1 slot a switches (foe b fainted); 2 slot a passes; 3 slot a Helping Hands."""
    B = 4
    ids = torch.zeros(B, 12, N_IDS, dtype=torch.long)
    floats = torch.zeros(B, 12, N_POKEMON_FLOATS)
    af = torch.zeros(B, 2, 40, N_ACTION_FEATURES)
    floats[:, [0, 1, 6, 7], PF["present"]] = 1
    hp = torch.tensor([[0.4, 0.8], [1.0, 0.5], [1.0, 0.5], [1.0, 0.6]])
    floats[:, 6:8, PF["hp"]] = hp
    floats[1:3, 7, PF["fainted"]] = 1  # hp left in on purpose: fainted masks it
    hh = model.helping_hand
    ids[0, 0, I_MOVE0 + 3] = hh  # known but not chosen
    ids[3, 0, I_MOVE0 + 2] = hh
    # Row 0: slot a's Thunderbolt (code(0, 3)) KOs foe a for sure.
    af[0, 0, code(0, 3), A["foe_a_damage"]], af[0, 0, code(0, 3), A["foe_a_ko"]] = 0.4, 1.0
    af[0, 1, code(1, 3), A["foe_a_damage"]], af[0, 1, code(1, 3), A["foe_a_ko"]] = 0.3, 0.2
    af[0, 1, code(1, 4), A["foe_b_damage"]] = 0.5
    # Rows 1-2: slot a's move rows carry damage, but slot a switches / passes.
    for row in (1, 2):
        af[row, 0, :, A["foe_a_damage"]:A["foe_b_ko"] + 1] = 0.9
        af[row, 1, code(0, 3), A["foe_a_damage"]] = 0.6
        af[row, 1, code(0, 4), A["foe_b_damage"]] = 0.7
    # Row 3: slot b's attacks under Helping Hand.
    af[3, 1, code(0, 3), A["foe_a_damage"]] = 0.5
    af[3, 1, code(0, 4), A["foe_b_damage"]] = 0.5
    t = dict(ids=ids, floats=floats, field=torch.zeros(B, N_FIELD_FLOATS), action_features=af,
             token_features=torch.zeros(B, 12, N_TOKEN_FEATURES))
    a0 = torch.tensor([code(0, 3), 41, 46, code(2, 0)])
    return t, a0


def test_feature_values_on_a_hand_built_batch():
    model = Model(load_dex(), width=32, layers=1, joint_features=True)
    assert model.helping_hand > 0
    t, a0 = hand_built(model)
    state = model.trunk(*(t[k] for k in KEYS))
    x = model.partner_features(state, torch.nn.functional.one_hot(a0, 47).float())
    # Row 0: foe a is gone after slot a, so slot b's damage on it is wasted.
    sure = x[0]
    assert (sure[:, J["left_a"]] == 0).all()
    torch.testing.assert_close(sure[:, J["left_b"]], torch.full((40,), 0.8))  # slot a did not touch foe b
    hit_a = sure[code(1, 3)]
    assert hit_a[J["wasted_a"]] == pytest.approx(0.3)
    assert hit_a[J["damage_a"]] == pytest.approx(0.4) and hit_a[J["overkill_a"]] == pytest.approx(0.3)
    assert hit_a[J["ko_a"]] == pytest.approx(1.0) and hit_a[J["finish_a"]] == pytest.approx(1.0)
    hit_b = sure[code(1, 4)]
    assert hit_b[J["damage_b"]] == pytest.approx(0.5) and hit_b[J["overkill_b"]] == 0
    assert hit_b[J["ko_b"]] == 0 and hit_b[J["wasted_b"]] == 0 and hit_b[J["finish_b"]] == 0
    assert (sure[:, J["helping_hand"]] == 0).all()  # Helping Hand known, a different move chosen
    # Rows 1-2: a switch or PASS gives no slot-a terms; the fainted foe b gives nothing.
    torch.testing.assert_close(x[1], x[2])
    alone = x[1, code(0, 3)]
    assert alone[J["left_a"]] == pytest.approx(1.0) and alone[J["damage_a"]] == pytest.approx(0.6)
    assert alone[J["overkill_a"]] == 0 and alone[J["ko_a"]] == 0 and alone[J["wasted_a"]] == 0
    assert alone[J["finish_a"]] == 0  # 0.6 against 1.0
    b_columns = [J[f"{name}_b"] for name in ("left", "damage", "overkill", "ko", "finish", "wasted")]
    assert not x[1][:, b_columns].any()
    assert not x[1:3, :, J["helping_hand"]].any()
    # Row 3: Helping Hand multiplies slot b's damage by 1.5 in the combined terms.
    boosted = x[3]
    assert (boosted[:, J["helping_hand"]] == 1).all()
    assert boosted[code(0, 3), J["damage_a"]] == pytest.approx(0.75)
    assert boosted[code(0, 3), J["ko_a"]] == 0
    assert boosted[code(0, 4), J["damage_b"]] == pytest.approx(0.6)
    assert boosted[code(0, 4), J["overkill_b"]] == pytest.approx(0.15)
    assert boosted[code(0, 4), J["ko_b"]] == pytest.approx(1.0)  # 0.75 against 0.6: B_ko raised to the boosted chance
    assert float(ko_reach(torch.tensor(0.5), torch.tensor(0.6))) == 0  # it was 0 without Helping Hand


def test_finish_counts_slot_a_chip_damage():
    model = Model(load_dex(), width=32, layers=1, joint_features=True)
    t, _ = hand_built(model)
    af = t["action_features"]
    af[0, 0, code(0, 3), A["foe_a_ko"]] = 0.0
    t["floats"][0, 6, PF["hp"]] = 1.0
    af[0, 0, code(0, 3), A["foe_a_damage"]] = 0.6  # slot a: 60%, no KO alone
    af[0, 1, code(1, 3), A["foe_a_damage"]], af[0, 1, code(1, 3), A["foe_a_ko"]] = 0.5, 0.0  # slot b: 50%
    state = model.trunk(*(t[k] for k in KEYS))
    x = model.partner_features(state, torch.nn.functional.one_hot(torch.tensor([code(0, 3), 46, 46, 46]), 47).float())
    row = x[0, code(1, 3)]
    assert row[J["ko_a"]] == 0  # neither KOs alone
    assert row[J["finish_a"]] == pytest.approx(1.0)  # 50% against the 40% slot a leaves
    assert row[J["left_a"]] == pytest.approx(0.4) and row[J["overkill_a"]] == pytest.approx(0.1)


@pytest.mark.parametrize("joint", [False, True])
def test_from_state_and_load_policy_round_trip(joint, tmp_path):
    import evaluate

    dex = load_dex()
    torch.manual_seed(0)
    model = Model(dex, width=32, layers=1, joint_features=joint)
    if joint:
        randomize_joint(model)
    again = Model.from_state(dex, model.state_dict(), width=32, layers=1)
    assert again.joint_features == joint
    path = tmp_path / "snapshot.pt"
    torch.save({"model": model.state_dict(), "width": 32, "layers": 1}, path)
    agent = evaluate.load_policy(path, dex, torch.device("cpu"), np.random.default_rng(0))
    assert agent.model.joint_features == joint
    t = observation()
    a0 = torch.tensor([code(0, 3), 46])
    for loaded in (again, agent.model):
        assert list(loaded.state_dict()) == list(model.state_dict())
        for x, y in zip(policy(model.eval(), t, a0), policy(loaded, t, a0)):
            torch.testing.assert_close(x, y, rtol=0, atol=0)


def test_joint_model_needs_the_per_foe_columns():
    with pytest.raises(ValueError):
        Model(load_dex(), width=32, layers=1, action_features=A["foe_b_ko"], joint_features=True)


def test_ppo_ratio_is_one_for_the_behaviour_policy():
    """The learner recomputes the joint features from the stored observation and a0, so the ratio starts at 1."""
    from train_selfplay import ppo_loss

    dex = load_dex()
    torch.manual_seed(0)
    model = Model(dex, width=32, layers=1, joint_features=True)
    randomize_joint(model)
    t = observation()
    a0, a1 = torch.tensor([code(0, 3), 46]), torch.tensor([code(1, 4), 46])
    mask = torch.ones(2, 47, dtype=torch.bool)
    old_logp = torch.zeros(2)
    with torch.no_grad():  # scored one row at a time, unlike the learner's batch
        for r in range(2):
            lp0, lp1 = policy(model, {k: v[r:r + 1] for k, v in t.items()}, a0[r:r + 1])
            old_logp[r] = lp0[0, a0[r]] + lp1[0, a1[r]]
    batch = dict(t, foe_match=torch.full((2, 6), -1, dtype=torch.long))
    _, terms = ppo_loss(model, batch, t["ids"].flip(0), t["floats"].flip(0)[:, 0:6], a0, a1, mask, mask, old_logp,
                        torch.ones(2), torch.zeros(2), 0.5, 0.01, 0.0, 0.2)
    assert abs(float(terms[5])) < 1e-6 and float(terms[6]) == 0  # KL and clip fraction


def test_actors_build_joint_models_and_store_matching_log_probs():
    from training.actors import Actors
    from training.league import SELF

    dex = load_dex()
    torch.manual_seed(0)
    model = Model(dex, width=32, layers=1, joint_features=True)
    randomize_joint(model)
    with Actors(2, 6, [POOL], seed=3, workers=1, model=model) as actors:
        actors.set_weights("learner", model.state_dict())
        actors.set_weights("opponent", model.state_dict())
        actors.start(0, np.full(2, SELF, dtype=np.int8), ko_bonus=0.0)
        actors.wait()
        data = {k: torch.from_numpy(v.copy()) for k, v in actors.buffers[0].arrays.items()}
    flat = {k: v.flatten(0, 1) for k, v in data.items() if v.dim() >= 2}
    with torch.no_grad():
        state = model.eval().trunk(*(flat[k] for k in KEYS))
        lp0, vectors0 = model.slot0(state, flat["mask0"])
        lp1 = model.slot1(state, vectors0, flat["a0"], flat["mask1"])
    assert (flat["a0"] < 40).any()  # some rows had slot a attack
    rows = torch.arange(len(flat["a0"]))
    logp = lp0[rows, flat["a0"]] + lp1[rows, flat["a1"]]
    torch.testing.assert_close(logp, flat["logp"], rtol=0, atol=2e-4)
