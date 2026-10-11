"""Rust-only switching vision, entry hazards and Protect choice priors."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest

nb = pytest.importorskip("numbrion")
if not hasattr(nb, "ObsEncoder"):
    pytest.skip("wheel lacks the Rust encoder", allow_module_level=True)

from training.damage import ACTION_COLUMNS, TOKEN_COLUMNS
from training.dex import Dex, load_dex
from training.encoder import LEGACY_ACTION_FEATURES, LEGACY_TOKEN_FEATURES, PF, RustEncoder

A = {name: LEGACY_ACTION_FEATURES + i for i, name in enumerate(ACTION_COLUMNS)}
T = {name: LEGACY_TOKEN_FEATURES + i for i, name in enumerate(TOKEN_COLUMNS)}


def code(move, target=3, tera=False):
    return move * 10 + target * 2 + int(tera)


def encode(extra=(), *, foe="Inteleon", moves=None, attack=150, item="", ability="blaze", hp="250/250",
           active_ability="blaze", dex=None):
    moves = moves or ["thunderbolt", "energyball", "flamethrower", "protect"]
    team = [("Infernape", moves, active_ability), ("Dipplin", ["energyball", "protect"], "supersweetsyrup"),
            ("Jolteon", ["thunderbolt"], "voltabsorb"), ("Flareon", ["flamethrower"], "flashfire"),
            ("Talonflame", ["bravebird"], ability), ("Arcanine", ["flamethrower"], ability)]
    pokemon = [{"ident": f"p1: {name}", "details": f"{name}, L80", "condition": hp if i >= 4 else "250/250",
                "active": i < 2, "stats": {"atk": attack, "spa": attack, "def": 150, "spd": 150, "spe": 150},
                "moves": known, "item": item if i >= 4 else "", "ability": skill, "teraType": "Fire"}
               for i, (name, known, skill) in enumerate(team)]
    active = [{"moves": [{"id": m, "pp": 8, "maxpp": 8, "disabled": False} for m in known],
               "canTerastallize": "Fire"} for _, known, _ in team[:2]]
    req = json.dumps({"active": active, "side": {"id": "p1", "pokemon": pokemon}})
    lines = ["|switch|p1a: Infernape|Infernape, L80|250/250", "|switch|p1b: Dipplin|Dipplin, L80|250/250",
             f"|switch|p2a: {foe}|{foe}, L80|100/100", "|switch|p2b: Ditto|Ditto, L80|100/100",
             "|turn|1", *extra]
    if not lines[-1].startswith("|turn|"):
        lines.append("|turn|2")
    turn = int(lines[-1].split("|")[2])
    enc = RustEncoder(2, dex=dex)
    enc.encode_batch([{"battle_id": 1, "log": lines, "request": req, "turn": turn},
                      {"battle_id": 1, "log": [], "request": "", "turn": turn}])
    return enc.buffers


@pytest.fixture(scope="module")
def balloon_dex(tmp_path_factory):
    """The random-doubles dex omits Air Balloon; include it in a test-only vocabulary."""
    path = tmp_path_factory.mktemp("balloon-dex") / "dex.json"
    data = json.loads(load_dex().path.read_text(encoding="utf-8"))
    if not any(item["id"] == "airballoon" for item in data["items"]):
        data["items"].append({"id": "airballoon", "name": "Air Balloon"})
    path.write_text(json.dumps(data), encoding="utf-8")
    return Dex(path)


def protect_lines(foe="Inteleon", success=False):
    return [f"|move|p2a: {foe}|Protect|p2a: {foe}",
            f"|-singleturn|p2a: {foe}|Protect" if success else f"|-fail|p2a: {foe}", "|turn|2"]


def test_bench_offense_and_actives_match_legacy():
    b = encode()
    tf, af = b.token_features[0], b.action_features[0]
    assert tf[2, T["offense_a"]] > tf[3, T["offense_a"]] > 0  # Electric vs resisted Fire on Water
    assert not tf[6:, T["offense_a"]:T["offense_ko_b"] + 1].any()
    for slot in range(2):
        for f in range(2):
            assert tf[slot, T[f"offense_{'ab'[f]}"]] == pytest.approx(
                max(af[slot, code(m, 3 + f), 0] for m in range(4)), abs=1e-5)
            assert tf[slot, T[f"offense_ko_{'ab'[f]}"]] == pytest.approx(
                max(af[slot, code(m, 3 + f), 2] for m in range(4)), abs=1e-5)


def test_spread_offense_uses_legacy_share():
    b = encode(["|-damage|p2b: Ditto|0 fnt", "|faint|p2b: Ditto", "|turn|2"], moves=["surf"])
    assert b.token_features[0, 0, T["offense_a"]] == pytest.approx(b.action_features[0, 0, code(0, 2), 0], abs=1e-5)
    assert b.token_features[0, 0, T["offense_ko_a"]] == pytest.approx(b.action_features[0, 0, code(0, 2), 2], abs=1e-5)
    assert not b.token_features[0, :6, T["offense_b"]].any()


def test_entry_stealth_rock_uses_type_chart_and_known_immunities():
    rock = ["|-sidestart|p1: Player|move: Stealth Rock"]
    b = encode(rock)
    assert b.token_features[0, 4, T["entry_damage"]] == pytest.approx(0.5)  # Fire/Flying: 4x
    assert b.token_features[0, 5, T["entry_damage"]] == pytest.approx(0.25)  # Fire: 2x
    assert not b.token_features[0, 6:, T["entry_damage"]].any()
    for own in ({"item": "heavydutyboots"}, {"ability": "magicguard"}):
        assert not encode(rock, **own).token_features[0, 4:6, T["entry_damage"]].any()
    assert encode(rock, hp="10/250").token_features[0, 4, T["entry_damage"]] == pytest.approx(0.04)
    assert not encode(["|-sidestart|p2: Foe|move: Stealth Rock"]).token_features[0, :6, T["entry_damage"]].any()


@pytest.mark.parametrize("layers,damage", [(1, 1 / 8), (2, 1 / 6), (3, 1 / 4)])
def test_spikes_grounding_and_gravity(layers, damage, balloon_dex):
    spikes = ["|-sidestart|p1: Player|Spikes"] * layers
    b = encode(spikes)
    assert b.token_features[0, 4, T["entry_damage"]] == 0  # Flying
    assert b.token_features[0, 5, T["entry_damage"]] == pytest.approx(damage)
    assert encode(spikes, item="airballoon", dex=balloon_dex).token_features[0, 5, T["entry_damage"]] == 0
    assert encode(spikes, ability="levitate").token_features[0, 5, T["entry_damage"]] == 0
    gravity = encode(spikes + ["|-fieldstart|move: Gravity"], item="airballoon", ability="levitate", dex=balloon_dex)
    np.testing.assert_allclose(gravity.token_features[0, 4:6, T["entry_damage"]], damage)


def test_protect_prior_by_belief_reveal_and_threat():
    plain = encode()
    af = plain.action_features[0, 0, code(0)]
    assert af[A["protect_chance_a"]] == 0  # Inteleon's belief has no Protect
    assert af[A["protect_chance_b"]] == 0
    np.testing.assert_array_equal(plain.action_features[..., A["damage_after_protect"]], plain.action_features[..., 0])
    np.testing.assert_array_equal(plain.action_features[..., A["ko_after_protect"]], plain.action_features[..., 2])
    assert encode(foe="Gengar").token_features[0, 6, T["protect_now"]] > 0  # belief alone
    low = encode(protect_lines(), attack=1)
    high = encode(protect_lines(), attack=500)
    assert high.token_features[0, 6, T["offense_ko_a"]] == 0  # foe offense columns stay zero
    assert high.token_features[0, 0, T["offense_ko_a"]] > low.token_features[0, 0, T["offense_ko_a"]]
    assert high.action_features[0, 0, code(0), A["protect_chance_a"]] > low.action_features[0, 0, code(0), A["protect_chance_a"]] > 0
    tf = high.token_features[0]
    damage = min(1, sum(tf[s, T["offense_a"]] for s in range(2)))
    ko = 1 - np.prod([1 - tf[s, T["offense_ko_a"]] for s in range(2)])
    prior = min(0.8, 0.1 + 0.2 * damage + 0.3 * ko)
    assert tf[6, T["protect_now"]] == pytest.approx(prior)
    assert not tf[:6, T["protect_now"]].any() and not tf[8:, T["protect_now"]].any()
    af = high.action_features[0, 0, code(0)]
    assert af[A["damage_after_protect"]] == pytest.approx(af[0] * (1 - prior))
    assert af[A["damage_after_protect"]] < af[0]
    assert af[A["ko_after_protect"]] == pytest.approx(af[2] * (1 - prior))
    again = encode(protect_lines(success=True), attack=500)
    assert again.floats[0, 6, PF["protect_last"]] == 1
    assert again.token_features[0, 6, T["protect_now"]] == pytest.approx(prior / 3)
    assert again.action_features[0, 0, code(0), A["protect_chance_a"]] == pytest.approx(prior / 3)
    np.testing.assert_array_equal(high.action_features[0, :, 0::2, A["protect_chance_a"]:],
                                  high.action_features[0, :, 1::2, A["protect_chance_a"]:])
    assert high.action_features[0, 0, code(0, 4), A["protect_chance_a"]] == 0  # targets only foe b


def test_protect_bypass_and_spread_discount():
    extra = protect_lines()
    b = encode(extra, moves=["surf", "feint", "thunderpunch"])
    af = b.action_features[0, 0]
    prior = b.token_features[0, 6, T["protect_now"]]
    assert af[code(1), A["protect_chance_a"]] == 0  # Feint bypasses Protect
    assert af[code(1), A["damage_after_protect"]] == af[code(1), 0]
    assert af[code(0, 2), A["protect_chance_a"]] == pytest.approx(prior)
    assert af[code(0, 2), A["protect_chance_b"]] == 0
    assert 0 < af[code(0, 2), A["damage_after_protect"]] < af[code(0, 2), 0]
    unseen = encode(extra, moves=["thunderpunch"], active_ability="unseenfist")
    assert unseen.token_features[0, 6, T["protect_now"]] > 0
    punch = unseen.action_features[0, 0, code(0)]
    assert punch[A["protect_chance_a"]] == 0
    assert punch[A["damage_after_protect"]] == punch[0]


def test_spread_discounts_each_foes_legacy_share():
    extra = protect_lines() + ["|move|p2b: Ditto|Protect|p2b: Ditto", "|-fail|p2b: Ditto", "|turn|3"]
    b = encode(extra, moves=["surf"])
    tf, surf = b.token_features[0], b.action_features[0, 0, code(0, 2)]
    chances = [tf[6 + f, T["protect_now"]] for f in range(2)]
    assert all(chance > 0 for chance in chances)
    for f in range(2):
        assert surf[A[f"protect_chance_{'ab'[f]}"]] == pytest.approx(chances[f])
    # With Surf as the user's only move, offense holds exactly that foe's legacy share.
    dealt = [tf[0, T[f"offense_{slot}"]] for slot in "ab"]
    kos = [tf[0, T[f"offense_ko_{slot}"]] for slot in "ab"]
    assert surf[0] == pytest.approx(sum(dealt))
    assert surf[2] == pytest.approx(sum(kos))
    assert surf[A["damage_after_protect"]] == pytest.approx(sum(d * (1 - p) for d, p in zip(dealt, chances)))
    assert surf[A["ko_after_protect"]] == pytest.approx(sum(k * (1 - p) for k, p in zip(kos, chances)))
