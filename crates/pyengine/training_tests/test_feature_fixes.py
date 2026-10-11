"""Damage-feature fixes: a spread move's main target, Stellar STAB once terastallized, and fainted foes' attacks."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pytest

from training.encoder import PF, Encoder

SWITCH_IN = ["|switch|p1a: Infernape|Infernape, L85, M|250/250", "|switch|p1b: Talonflame|Talonflame, L88, M|200/200",
             "|switch|p2a: Inteleon|Inteleon, L78, F|100/100", "|switch|p2b: Gengar|Gengar, L80, M|100/100"]


def request(moves, tera_type="Fire", terastallized=""):
    team = [("Infernape", moves, "blaze", tera_type, 85, terastallized),
            ("Talonflame", ["bravebird", "protect"], "galewings", "Flying", 88, "")]
    pokemon = [{"ident": f"p1: {name}", "details": f"{name}, L{level}", "condition": "200/250", "active": True,
                "stats": {"atk": 250, "def": 180, "spa": 250, "spd": 180, "spe": 260}, "moves": known,
                "item": "lifeorb", "ability": ability, "teraType": kind, "terastallized": done}
               for name, known, ability, kind, level, done in team]
    active = [{"moves": [{"id": m, "pp": 8, "maxpp": 8, "disabled": False} for m in known]}
              for _, known, _, _, _, _ in team]
    return json.dumps({"active": active, "side": {"id": "p1", "pokemon": pokemon}})


def encode(lines, req):
    encoder = Encoder(2)
    turn = int(lines[-1].split("|")[2])
    encoder.encode_batch([{"battle_id": 1, "log": lines, "request": req, "turn": turn},
                          {"battle_id": 1, "log": [], "request": "", "turn": turn}])
    return encoder.buffers


def code(move_slot, target_code, tera=False):
    return move_slot * 10 + target_code * 2 + int(tera)


def test_spread_move_main_target_is_a_foe():
    """Earthquake also hits the Flying partner; its effectiveness columns describe the foes, not the immune ally."""
    b = encode(SWITCH_IN + ["|turn|1"], request(["earthquake", "flareblitz", "protect", "fakeout"]))
    quake = b.action_features[0, 0, code(0, 2)]
    assert quake[0] > 0  # damage to the foes
    assert quake[4] == pytest.approx(0) and quake[5] == 0  # neutral on Inteleon, not immune


def test_stellar_boost_stays_after_terastallizing():
    moves = ["flareblitz", "protect"]
    before = encode(SWITCH_IN + ["|turn|1"], request(moves, "Stellar"))
    after = encode(SWITCH_IN + ["|-terastallize|p1a: Infernape|Stellar", "|turn|1"],
                   request(moves, "Stellar", terastallized="Stellar"))
    assert after.floats[0, 0, PF["terastallized"]] == 1
    ratio = after.action_features[0, 0, code(0, 3), 0] / before.action_features[0, 0, code(0, 3), 0]
    assert ratio == pytest.approx(2.0 / 1.5, rel=1e-4)  # Fire STAB 1.5x -> Stellar 2x


def test_fainted_foe_threatens_nothing():
    moves = ["flareblitz", "protect"]
    alive = encode(SWITCH_IN + ["|turn|1"], request(moves))
    fainted = encode(SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Flare Blitz|p2a: Inteleon",
                                  "|-damage|p2a: Inteleon|0 fnt", "|faint|p2a: Inteleon", "|turn|2"], request(moves))
    assert alive.token_features[0, 0, 0] > 0 and alive.token_features[0, 0, 2] > 0
    assert fainted.floats[0, 6, PF["fainted"]] == 1
    assert fainted.token_features[0, 0, 0] == 0 and fainted.token_features[0, 0, 2] == 0  # from foe a
    assert fainted.token_features[0, 0, 1] == pytest.approx(alive.token_features[0, 0, 1])  # foe b unchanged
