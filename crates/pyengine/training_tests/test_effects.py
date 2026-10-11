"""The effect columns of the damage-calc features (Fake Out, flinches, Protect, Tera) and the tracker state they use."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

from training.damage import ACTION_COLUMNS, TOKEN_COLUMNS
from training.dex import load_dex
from training.encoder import LEGACY_ACTION_FEATURES, LEGACY_TOKEN_FEATURES, PF, Encoder
from training.model import Model

A = {name: LEGACY_ACTION_FEATURES + i for i, name in enumerate(ACTION_COLUMNS)}
T = {name: LEGACY_TOKEN_FEATURES + i for i, name in enumerate(TOKEN_COLUMNS)}
SWITCH_IN = ["|switch|p1a: Infernape|Infernape, L85, M|250/250", "|switch|p1b: Dipplin|Dipplin, L90|200/200",
             "|switch|p2a: Inteleon|Inteleon, L78, F|100/100", "|switch|p2b: Gengar|Gengar, L80, M|100/100"]
INFERNAPE = ["fakeout", "closecombat", "protect", "flareblitz"]  # move slots 0-3


def request(tera=True, moves=INFERNAPE, ability="blaze", tera_type="Fire", spe=260):
    """Our side: slot a is Infernape with ``moves``, ``ability`` and ``tera_type``; slot b is Dipplin."""
    team = [("Infernape", moves, ability, tera_type, 85, spe),
            ("Dipplin", ["protect", "dracometeor"], "supersweetsyrup", "Dragon", 90, 100)]
    pokemon = [{"ident": f"p1: {name}", "details": f"{name}, L{level}", "condition": "200/250", "active": True,
                "stats": {"atk": 250, "def": 180, "spa": 250, "spd": 180, "spe": speed}, "moves": known,
                "item": "lifeorb", "ability": skill, "teraType": kind, "terastallized": ""}
               for name, known, skill, kind, level, speed in team]
    active = [{"moves": [{"id": m, "pp": 8, "maxpp": 8, "disabled": False} for m in known],
               **({"canTerastallize": kind} if tera else {})} for _, known, _, kind, _, _ in team]
    return json.dumps({"active": active, "side": {"id": "p1", "pokemon": pokemon}})


def encode(turns, **own):
    """Feed the protocol lines of all ``turns`` (each ends with its ``|turn|`` line) and return the buffers; ``own``
    goes to ``request``."""
    encoder = Encoder(2)
    lines = [line for turn in turns for line in turn]
    turn = int(lines[-1].split("|")[2])
    encoder.encode_batch([{"battle_id": 1, "log": lines, "request": request(**own), "turn": turn},
                          {"battle_id": 1, "log": [], "request": "", "turn": turn}])
    return encoder.buffers


def switch_in(foe_a="Inteleon, L78, F", foe_b="Gengar, L80, M"):
    return SWITCH_IN[:2] + [f"|switch|p2a: {foe_a.split(',')[0]}|{foe_a}|100/100",
                            f"|switch|p2b: {foe_b.split(',')[0]}|{foe_b}|100/100"]


def code(move_slot, target_code, tera=False):
    """Target codes: 0 own slot b, 1 own slot a, 2 none, 3 foe slot a, 4 foe slot b."""
    return move_slot * 10 + target_code * 2 + int(tera)


def test_tracker_counts_cant_as_a_move_action_and_failed_protect_as_no_protect():
    turn1 = SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Protect|p1a: Infernape", "|-singleturn|p1a: Infernape|Protect",
                         "|cant|p1b: Dipplin|flinch",
                         "|cant|p2a: Inteleon|ability: Armor Tail|Encore|[of] p1b: Dipplin", "|turn|2"]
    b = encode([turn1])
    floats = b.floats[0]
    assert floats[0, PF["protect_last"]] == 1 and floats[0, PF["fresh"]] == 0
    assert floats[1, PF["fresh"]] == 0  # flinched: its first move action is used up
    assert floats[6, PF["fresh"]] == 1  # Armor Tail's holder did not act
    turn2 = ["|move|p1a: Infernape|Protect||[still]", "|-fail|p1a: Infernape", "|turn|3"]
    assert encode([turn1, turn2]).floats[0, 0, PF["protect_last"]] == 0  # the failed Protect reset the counter


def test_fake_out_works_only_on_the_first_turn_and_flinches_what_can_flinch():
    b = encode([SWITCH_IN + ["|turn|1"]])
    af, tf = b.action_features[0, 0], b.token_features[0]
    at_inteleon, at_gengar = af[code(0, 3)], af[code(0, 4)]
    assert at_inteleon[A["fail"]] == 0 and at_inteleon[A["flinch_a"]] > 0.8 and at_inteleon[A["flinch_b"]] == 0
    assert at_inteleon[A["stop_damage"]] > 0
    assert at_gengar[A["flinch_b"]] == 0  # Ghost: Fake Out does not touch it
    assert tf[0, T["fake_out_ready"]] == 1 and tf[1, T["fake_out_ready"]] == 0  # Dipplin has no Fake Out
    assert tf[7, T["flinchable"]] == 0 and tf[6, T["flinchable"]] > 0.8
    assert tf[6, T["fake_out_threat"]] > 0.8  # our fresh Fake Out user threatens Inteleon
    # Close Combat cannot flinch; Fake Out with Tera has the same effect columns.
    assert af[code(1, 3), A["flinch_a"]] == 0
    np.testing.assert_array_equal(af[code(0, 3, True), A["fail"]:A["foe_protect_b"] + 1],
                                  at_inteleon[A["fail"]:A["foe_protect_b"] + 1])

    later = encode([SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Close Combat|p2a: Inteleon", "|turn|2"]])
    af, tf = later.action_features[0, 0], later.token_features[0]
    assert af[code(0, 3), A["fail"]] == 1 and af[code(0, 3), A["flinch_a"]] == 0 and af[code(0, 3), A["stop_damage"]] == 0
    assert tf[0, T["fake_out_ready"]] == 0 and tf[6, T["fake_out_threat"]] == 0
    assert af[code(1, 3), A["fail"]] == 0

    terrain = encode([SWITCH_IN + ["|-fieldstart|move: Psychic Terrain", "|turn|1"]])
    af = terrain.action_features[0, 0]
    assert af[code(0, 3), A["fail"]] == 1 and af[code(0, 3), A["flinch_a"]] == 0  # grounded Inteleon is protected


def test_protect_columns_follow_the_counter():
    b = encode([SWITCH_IN + ["|turn|1"]])
    protect = b.action_features[0, 0, code(2, 2)]
    assert protect[A["protect"]] == 1 and protect[A["protect_damage"]] > 0
    assert b.action_features[0, 0, code(1, 3), A["protect"]] == 0
    again = encode([SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Protect|p1a: Infernape",
                                 "|-singleturn|p1a: Infernape|Protect", "|turn|2"]])
    protect = again.action_features[0, 0, code(2, 2)]
    assert protect[A["protect"]] == pytest.approx(1 / 3)
    assert protect[A["protect_damage"]] == pytest.approx(b.action_features[0, 0, code(2, 2), A["protect_damage"]] / 3)

    # A foe that protected last turn can Protect again one time in three, the turn after at full chance.
    foe_turn = ["|move|p2a: Inteleon|Protect|p2a: Inteleon", "|-singleturn|p2a: Inteleon|Protect"]
    after = encode([SWITCH_IN + ["|turn|1"] + foe_turn + ["|turn|2"]])
    assert after.action_features[0, 0, code(1, 3), A["foe_protect_a"]] == pytest.approx(1 / 3)
    assert after.action_features[0, 0, code(1, 3), A["foe_protect_b"]] == 0  # aimed at Inteleon only
    assert after.token_features[0, 6, T["protect_ready"]] == pytest.approx(1 / 3)
    rested = encode([SWITCH_IN + ["|turn|1"] + foe_turn + ["|turn|2", "|turn|3"]])
    assert rested.action_features[0, 0, code(1, 3), A["foe_protect_a"]] == pytest.approx(1)


def test_tera_columns_only_on_tera_codes():
    b = encode([SWITCH_IN + ["|turn|1"]])
    af = b.action_features[0, 0]
    flare = af[code(3, 3, True)]
    assert flare[A["tera_damage"]] > 0  # Fire Tera: Flare Blitz goes from 1.5x to 2x STAB
    assert af[code(1, 3, True), A["tera_damage"]] == 0  # Close Combat is not Fire
    tera_columns = [A[k] for k in ("tera_damage", "tera_ko", "tera_threat", "tera_threat_ko")]
    assert not af[0::2][:, tera_columns].any()  # codes without Tera
    np.testing.assert_array_equal(af[code(3, 3), :LEGACY_ACTION_FEATURES], flare[:LEGACY_ACTION_FEATURES])
    assert (af[1::2, A["tera_threat"]] == af[1, A["tera_threat"]]).all()  # the user's defence, same for every code
    no_tera = encode([SWITCH_IN + ["|turn|1"]], tera=False).action_features[0, 0]
    assert not no_tera[:, tera_columns].any()


def test_models_read_only_the_columns_they_were_built_with():
    dex = load_dex()
    torch.manual_seed(0)
    b = encode([SWITCH_IN + ["|turn|1"]])
    t = {k: torch.from_numpy(getattr(b, k)) for k in ("ids", "floats", "field", "action_features", "token_features")}
    mask = torch.ones(2, 47, dtype=torch.bool)

    def logits(model, action_features, token_features):
        state = model.trunk(t["ids"], t["floats"], t["field"], action_features, token_features)
        return model.slot0(state, mask)[0]

    old = Model.from_state(dex, Model(dex, width=32, layers=1, action_features=LEGACY_ACTION_FEATURES,
                                      token_features=LEGACY_TOKEN_FEATURES).state_dict(), width=32, layers=1)
    assert (old.n_action_features, old.n_token_features) == (LEGACY_ACTION_FEATURES, LEGACY_TOKEN_FEATURES)
    noisy_a, noisy_t = t["action_features"].clone(), t["token_features"].clone()
    noisy_a[..., LEGACY_ACTION_FEATURES:] = torch.rand_like(noisy_a[..., LEGACY_ACTION_FEATURES:])
    noisy_t[..., LEGACY_TOKEN_FEATURES:] = torch.rand_like(noisy_t[..., LEGACY_TOKEN_FEATURES:])
    torch.testing.assert_close(logits(old, t["action_features"], t["token_features"]), logits(old, noisy_a, noisy_t))
    new = Model(dex, width=32, layers=1)
    assert not torch.allclose(logits(new, t["action_features"], t["token_features"]), logits(new, noisy_a, noisy_t))


def test_action_usage_reports_fake_out_and_protect_use():
    from training.diagnostics import ActionUsage
    from training.encoder import I_MOVE0, N_ACTION_FEATURES, N_IDS
    dex = load_dex()
    m = dex.move_index
    rows = 2
    ids = np.zeros((1, rows, 12, N_IDS), dtype=np.int64)
    ids[0, :, 0:2, I_MOVE0:I_MOVE0 + 4] = [m["protect"], m["fakeout"], m["thunderbolt"], m["swordsdance"]]
    mask = np.ones((1, rows, 47), dtype=bool)
    features = np.zeros((1, rows, 2, 40, N_ACTION_FEATURES), dtype=np.float32)
    features[0, 0, :, 10:20, A["flinch_a"]] = 1  # row 0: Fake Out (move slot 1) flinches foe a
    features[0, 1, :, 10:20, A["fail"]] = 1  # row 1: it no longer works ...
    floats = np.zeros((1, rows, 12, len(PF)), dtype=np.float32)
    floats[0, 0, 0:2, PF["fresh"]] = 1  # ... because only row 0's actives are fresh
    features[0, 1, 1, 0:10, A["protect"]] = 1 / 3  # row 1, slot b: Protect right after a successful one
    data = dict(a0=np.array([[code(1, 3), code(1, 3)]]), a1=np.array([[code(2, 4), code(0, 2)]]), mask0=mask,
                mask1=mask, ids=ids, floats=floats, train=np.ones((1, rows), dtype=bool), action_features=features)
    out = ActionUsage(dex)(data)
    assert out["use_decisions"] == 4 and out["fake_out_ready"] == pytest.approx(0.5)
    assert out["use_fake_out_when_ready"] == pytest.approx(0.5)  # row 0 slot a used it, slot b did not
    assert out["fake_out_stale"] == pytest.approx(0.5) and out["protect_repeat"] == pytest.approx(1.0)


def test_protect_counts_only_what_it_blocks():
    """Unseen Fist contact moves hit through Protect, so Protect is worth less against Urshifu."""
    b = encode([switch_in(foe_a="Urshifu-Rapid-Strike, L76, M") + ["|turn|1"]])
    threat = b.token_features[0, 0, 0:2]  # Infernape's expected damage from foe a (Urshifu) and foe b (Gengar)
    protect = b.action_features[0, 0, code(2, 2)]
    assert threat[0] > 0.2 and protect[A["protect"]] == 1
    assert protect[A["protect_damage"]] == pytest.approx(min(threat.sum(), 1) - min(threat[0], 1), abs=1e-5)


def test_a_faster_foe_fake_out_comes_first():
    slow = encode([switch_in(foe_a="Rillaboom, L80, M") + ["|turn|1"]], spe=50).action_features[0, 0, code(0, 3)]
    fast = encode([switch_in(foe_a="Rillaboom, L80, M") + ["|turn|1"]], spe=400).action_features[0, 0, code(0, 3)]
    assert fast[A["flinch_a"]] == pytest.approx(1)
    assert slow[A["flinch_a"]] < 0.5  # Rillaboom (likely Fake Out) is faster and goes first


def test_attacker_abilities_change_flinches():
    moves = ["ironhead", "rockslide", "protect", "fakeout"]
    plain = encode([switch_in() + ["|turn|1"]], moves=moves, ability="blaze").action_features[0, 0]
    assert plain[code(0, 3), A["flinch_a"]] == pytest.approx(0.3)  # faster Iron Head: 30%
    assert plain[code(1, 2), A["flinch_a"]] == pytest.approx(0.3 * 0.9)  # Rock Slide: 90% accuracy
    serene = encode([switch_in() + ["|turn|1"]], moves=moves, ability="serenegrace").action_features[0, 0]
    assert serene[code(0, 3), A["flinch_a"]] == pytest.approx(0.6)
    sheer = encode([switch_in() + ["|turn|1"]], moves=moves, ability="sheerforce").action_features[0, 0]
    assert sheer[code(0, 3), A["flinch_a"]] == 0 and sheer[code(1, 2), A["flinch_a"]] == 0
    focus = switch_in() + ["|-ability|p2a: Inteleon|Inner Focus", "|turn|1"]
    assert encode([focus], moves=moves).action_features[0, 0, code(3, 3), A["flinch_a"]] == 0
    breaker = encode([focus], moves=moves, ability="moldbreaker").action_features[0, 0]
    assert breaker[code(3, 3), A["flinch_a"]] == pytest.approx(1)  # Mold Breaker ignores Inner Focus
    from training.damage import DamageFeatures
    dex = load_dex()
    assert float(DamageFeatures(dex).move_flinch[dex.move_index["triplearrows"]]) == pytest.approx(0.3)


def test_tera_blast_and_stellar_on_tera_codes():
    moves = ["terablast", "protect", "flareblitz", "closecombat"]
    water = encode([switch_in() + ["|turn|1"]], moves=moves, tera_type="Water").action_features[0, 0]
    assert water[code(0, 4), 0] == 0  # Normal Tera Blast does not touch Gengar ...
    assert water[code(0, 4, True), A["tera_damage"]] > 0  # ... a Water one does
    stellar = encode([switch_in() + ["|turn|1"]], moves=moves, tera_type="Stellar").action_features[0, 0]
    assert stellar[code(2, 3, True), A["tera_damage"]] > 0  # Flare Blitz: STAB 1.5x -> 2x


def test_prankster_status_fails_on_dark_types():
    moves = ["thunderwave", "protect", "fakeout", "closecombat"]
    b = encode([switch_in(foe_b="Kingambit, L78, M") + ["|turn|1"]], moves=moves, ability="prankster")
    af = b.action_features[0, 0]
    assert af[code(0, 4), A["fail"]] == 1 and af[code(0, 3), A["fail"]] == 0  # Kingambit is Dark, Inteleon not
    plain = encode([switch_in(foe_b="Kingambit, L78, M") + ["|turn|1"]], moves=moves).action_features[0, 0]
    assert plain[code(0, 4), A["fail"]] == 0
