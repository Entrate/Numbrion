"""Isolation tests for the self-play pipeline (docs/training/TIPS.md rule 4)."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

import numbrion as nb
from training.damage import DamageFeatures
from training.dex import load_dex, to_id
from training.encoder import FF, I_ABILITY, I_CAND_MOVES, I_ITEM, I_MOVE0, I_SPECIES, I_TERA, K_MOVES, PF, Encoder
from training.model import TARGET_TOKENS, Model, bag
from training.ppo import PASS, gae, sample

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"


def request(names=("Dipplin", "Volbeat"), extra=()):
    pokemon = []
    for i, name in enumerate(list(names) + list(extra)):
        pokemon.append({"ident": f"p1: {name}", "details": f"{name}, L90", "condition": "100/200", "active": i < 2,
                        "stats": {"atk": 100, "def": 100, "spa": 100, "spd": 100, "spe": 100},
                        "moves": ["protect", "fakeout"], "item": "leftovers", "ability": "intimidate",
                        "teraType": "Water", "terastallized": ""})
    return json.dumps({"active": [{"moves": [{"id": "protect", "pp": 5, "maxpp": 10, "disabled": False},
                                             {"id": "fakeout", "pp": 10, "maxpp": 10, "disabled": True}]}] * 2,
                       "side": {"id": "p1", "pokemon": pokemon}})


def test_tracker_reads_public_protocol():
    dex = load_dex()
    encoder = Encoder(2)
    lines = [
        "|switch|p1a: Dipplin|Dipplin, L90|200/200", "|switch|p1b: Volbeat|Volbeat, L90|200/200",
        "|switch|p2a: Inteleon|Inteleon, L78, F|100/100", "|switch|p2b: Skuntank|Skuntank, L85, M|100/100",
        "|turn|1",
        "|move|p2a: Inteleon|Scald|p1a: Dipplin", "|-damage|p1a: Dipplin|100/200",
        "|-boost|p2b: Skuntank|atk|2", "|-unboost|p2a: Inteleon|spe|1",
        "|-damage|p1b: Volbeat|150/200|[from] item: Rocky Helmet|[of] p2b: Skuntank",
        "|-ability|p2a: Inteleon|Torrent", "|-terastallize|p2b: Skuntank|Poison",
        "|-sidestart|p2: Player 2|move: Tailwind", "|-weather|RainDance", "|move|p1a: Dipplin|Protect|p1a: Dipplin",
        "|turn|2",
    ]
    encoder.encode(0, {"battle_id": 1, "log": lines, "request": request(), "turn": 2})
    b = encoder.buffers
    inteleon, skuntank = b.ids[0, 6], b.ids[0, 7]
    assert inteleon[I_SPECIES] == dex.species_index["inteleon"]
    assert skuntank[I_SPECIES] == dex.species_index["skuntank"]
    assert inteleon[I_MOVE0] == dex.move_index["scald"]
    assert inteleon[I_ABILITY] == dex.ability_index["torrent"]
    assert skuntank[I_ITEM] == dex.item_index["rockyhelmet"]
    assert skuntank[I_TERA] == dex.type_idx("Poison")
    assert b.floats[0, 7, PF["boost_atk"]] == pytest.approx(2 / 6)
    assert b.floats[0, 6, PF["boost_spe"]] == pytest.approx(-1 / 6)
    assert b.floats[0, 7, PF["terastallized"]] == 1
    assert b.floats[0, 6, PF["active_a"]] == 1 and b.floats[0, 7, PF["active_b"]] == 1
    assert b.field[0, FF["foe_tailwind"]] == 1 and b.field[0, FF["rain"]] == 1
    # Own side: HP from the request, protect timing and PP/disabled from the active request.
    assert b.floats[0, 0, PF["hp"]] == pytest.approx(0.5)
    assert b.floats[0, 0, PF["protect_last"]] == 1
    assert b.floats[0, 0, PF["pp0"]] == pytest.approx(0.5) and b.floats[0, 0, PF["disabled1"]] == 1
    # A move unrevealed by Inteleon remains a belief candidate, not a known move.
    assert b.floats[0, 6, PF["moves_seen"]] == pytest.approx(0.25)
    assert b.floats[0, 6, PF["cand_move_p0"]] > 0


def test_broken_illusion_moves_observations_to_the_real_pokemon():
    dex = load_dex()
    # Case 1: the disguise's species was never really seen, so its record is a phantom and must disappear.
    encoder = Encoder(2)
    phantom = ["|switch|p2a: Inteleon|Inteleon, L78|100/100", "|switch|p2b: Raichu|Raichu, L85|100/100", "|turn|1",
               "|move|p2b: Raichu|Knock Off|p1a: Dipplin", "|-boost|p2b: Raichu|spa|1",
               "|-damage|p2b: Raichu|60/100", "|-damage|p2b: Raichu|48/100|[from] item: Life Orb",
               "|replace|p2b: Zoroark|Zoroark, L80, M",  # the engine's replace line carries no HP
               "|-end|p2b: Zoroark|Illusion", "|turn|2"]
    encoder.encode(0, {"battle_id": 1, "log": phantom, "request": request(), "turn": 2})
    b = encoder.buffers
    species = [int(x) for x in b.ids[0, 6:12, I_SPECIES]]
    assert dex.species_index["raichu"] not in species
    assert b.ids[0, 7, I_SPECIES] == dex.species_index["zoroark"]
    assert b.floats[0, 7, PF["hp"]] == pytest.approx(0.48) and b.floats[0, 7, PF["boost_spa"]] == pytest.approx(1 / 6)
    assert b.ids[0, 7, I_MOVE0] == dex.move_index["knockoff"]
    assert b.ids[0, 7, I_ITEM] == dex.item_index["lifeorb"] and b.ids[0, 7, I_ABILITY] == dex.ability_index["illusion"]
    assert b.floats[0, 6:12, PF["present"]].sum() == 2
    # Case 2: the real Raichu was seen before, so its record returns to what was known before the disguise.
    encoder = Encoder(2)
    seen = ["|switch|p2a: Inteleon|Inteleon, L78|100/100", "|switch|p2b: Raichu|Raichu, L85|100/100", "|turn|1",
            "|move|p2b: Raichu|Thunderbolt|p1a: Dipplin", "|-damage|p2b: Raichu|70/100",
            "|switch|p2b: Garchomp|Garchomp, L80|100/100", "|turn|2",
            "|switch|p2b: Raichu|Raichu, L85|70/100", "|move|p2b: Raichu|Night Daze|p1a: Dipplin",
            "|move|p2b: Raichu|Thunderbolt|p1a: Dipplin",  # already known for Raichu: used by Zoroark too
            "|-damage|p2b: Raichu|30/100", "|replace|p2b: Zoroark|Zoroark, L80, M", "|turn|3"]
    encoder.encode(0, {"battle_id": 1, "log": seen, "request": request(), "turn": 3})
    tracker = encoder.trackers[0]
    raichu, zoroark = tracker.mons[("p2", "Raichu")], tracker.mons[("p2", "Zoroark")]
    assert raichu.hp == pytest.approx(0.70) and raichu.moves == ["thunderbolt"]
    assert zoroark.hp == pytest.approx(0.30) and zoroark.moves == ["nightdaze", "thunderbolt"]
    assert tracker.active["p2b"] == ("p2", "Zoroark")


def test_switch_resets_boosts_and_foe_slots():
    encoder = Encoder(2)
    lines = ["|switch|p2a: Inteleon|Inteleon, L78|100/100", "|switch|p2b: Skuntank|Skuntank, L85|100/100", "|turn|1",
             "|-boost|p2a: Inteleon|spa|2", "|switch|p2a: Garchomp|Garchomp, L80|100/100", "|faint|p2b: Skuntank",
             "|turn|2"]
    encoder.encode(0, {"battle_id": 1, "log": lines, "request": request(), "turn": 2})
    b, dex = encoder.buffers, encoder.dex
    assert b.ids[0, 6, I_SPECIES] == dex.species_index["garchomp"]
    assert b.floats[0, 6, PF["boost_spa"]] == 0 and b.floats[0, 6, PF["fresh"]] == 1
    assert b.floats[0, 7, PF["fainted"]] == 1
    bench = [t for t in range(8, 12) if b.ids[0, t, I_SPECIES] == dex.species_index["inteleon"]]
    assert bench and b.floats[0, bench[0], PF["bench"]] == 1
    assert b.field[0, FF["foe_fainted"]] == pytest.approx(1 / 6)
    # A new battle id clears the tracker.
    encoder.encode(0, {"battle_id": 2, "log": [], "request": request(), "turn": 0})
    assert b.floats[0, 6:, PF["present"]].sum() == 0


def test_belief_narrows_with_reveals():
    dex = load_dex()
    sid = dex.species_id("Blastoise")
    fake_out = dex.move_index["fakeout"]
    moves, _, tera, _ = dex.belief(sid, frozenset({fake_out}), 0, 0)
    candidates = {m for m, p in moves if p > 0}
    support = next(s for s in dex.sets[sid] if s["role"] == "Doubles Bulky Attacker")
    assert candidates == {dex.move_index[m] for m in support["movepool"]} - {fake_out}
    assert sum(p for _, p in tera) == pytest.approx(1.0)


def test_action_code_layout_matches_engine():
    for code in range(40):
        decoded = nb.decode_action(code)
        assert decoded["slot"] == code // 10
        assert decoded["target"] + 2 == (code % 10) // 2
        assert decoded["tera"] == bool(code % 2)
    # Target code -> token: -2 own slot b, -1 own slot a, 0 none, +1 foe slot a, +2 foe slot b.
    assert TARGET_TOKENS == (1, 0, -1, 6, 7)
    assert nb.decode_action(40) == {"kind": "switch", "party": 0} and nb.decode_action(PASS) == {"kind": "pass"}


def battle_observations(seed, steps=6):
    env = nb.BatchEnv(1, [str(POOL)], seed=seed, threads=1, log=True)
    env.reset()
    history = []
    for _ in range(steps):
        history.append([env.observe(0, 0), env.observe(0, 1)])
        env.step(env.random_actions())
    return history


def test_policy_inputs_depend_only_on_own_view():
    """Row 0's arrays are identical whatever the opponent row sees; only the training-only pairing differs."""
    own = battle_observations(1)
    other = battle_observations(2)
    a, b = Encoder(2), Encoder(2)
    for step in range(len(own)):
        a.encode_batch([own[step][0], own[step][1]])
        b.encode_batch([own[step][0], other[step][1]])
    for key in ("ids", "floats", "field", "action_features", "token_features"):
        np.testing.assert_array_equal(getattr(a.buffers, key)[0], getattr(b.buffers, key)[0])


def test_damage_features_track_engine_damage():
    """Median actual/predicted damage of landed moves stays near 1 (the full check: scratch damage script)."""
    dex = load_dex()
    damage = DamageFeatures(dex)
    env = nb.BatchEnv(16, [str(POOL)], seed=5, threads=2, log=True)
    env.reset()
    encoder = Encoder(32)
    observations = [env.observe(e, s) for e in range(16) for s in (0, 1)]
    ratios = []
    for _ in range(60):
        b = encoder.encode_batch(observations)
        with torch.no_grad():
            action, token = damage(torch.from_numpy(b.ids), torch.from_numpy(b.floats), torch.from_numpy(b.field))
        assert torch.isfinite(action).all() and torch.isfinite(token).all()
        assert (action[..., 0] >= 0).all() and (action[..., 0] <= 2.0).all()
        before = [{k[1]: m.hp for k, m in encoder.trackers[2 * e].mons.items() if k[0] == "p2"} for e in range(16)]
        actions = env.random_actions(switch_prob=0.0, tera_prob=0.0)
        result = env.step(actions)
        observations = [env.observe(e, s) for e in range(16) for s in (0, 1)]
        for e in range(16):
            code = int(actions[e, 0, 0])
            if result["done"][e] or not 0 <= code < 40 or b.floats[2 * e, 0, PF["active_a"]] == 0:
                continue
            move = b.ids[2 * e, 0, I_MOVE0 + code // 10]
            if not move or dex.moves[move - 1]["category"] == "Status" or dex.move_accuracy[move] < 1:
                continue
            lines = observations[2 * e]["log"]
            name = dex.moves[move - 1]["name"]
            mine = [i for i, l in enumerate(lines) if l.startswith(f"|move|p1a: ") and f"|{name}|" in l]
            if not mine:
                continue
            dealt, events = 0.0, []
            for line in lines[mine[0] + 1:]:
                parts = line.split("|")
                if len(parts) < 2 or parts[1] in ("move", "upkeep", "turn", ""):
                    break
                events.append(parts[1])
                if parts[1] == "-damage" and parts[2].startswith("p2") and len(parts) == 4:
                    hp = parts[3].split()[0]
                    now = 0.0 if hp == "0" else int(hp.split("/")[0]) / int(hp.split("/")[1])
                    dealt += before[e].get(parts[2].split(": ", 1)[1], now) - now
            predicted = float(action[2 * e, 0, code, 0])
            if dealt > 0.02 and predicted > 0.02 and "-crit" not in events and "-activate" not in events:
                ratios.append(dealt / predicted)
    assert len(ratios) > 40
    assert 0.8 < float(np.median(ratios)) < 1.25


def test_bag_matches_embedding_sum():
    table = torch.randn(30, 8)
    ids = torch.tensor([[[3, 7, 0], [5, 0, 0]]])
    weights = torch.tensor([[[0.5, 1.0, 0.0], [2.0, 0.0, 0.0]]])
    expected = (table[ids] * weights.unsqueeze(-1)).sum(-2)
    torch.testing.assert_close(bag(ids, table, weights), expected)


def test_model_logits_ignore_oracle_inputs_and_respect_masks():
    dex = load_dex()
    torch.manual_seed(0)
    model = Model(dex, width=32, layers=1, heads=2)
    obs = battle_observations(3, steps=4)[-1]
    encoder = Encoder(2)
    b = encoder.encode_batch(obs)
    t = {k: torch.from_numpy(getattr(b, k)) for k in ("ids", "floats", "field", "action_features", "token_features")}
    state = model.trunk(t["ids"], t["floats"], t["field"], t["action_features"], t["token_features"])
    mask = torch.zeros(2, 47, dtype=torch.bool)
    mask[:, [3, 40, 41]] = True
    lp0, vectors0 = model.slot0(state, mask)
    assert torch.isfinite(lp0[:, [3, 40, 41]]).all() and (lp0[:, 0] < -1e8).all()
    picks = sample(lp0.detach().numpy(), np.random.default_rng(0))
    assert set(picks) <= {3, 40, 41}
    lp1 = model.slot1(state, vectors0, torch.tensor([3, 40]), mask)
    lp1_other = model.slot1(state, vectors0, torch.tensor([40, 3]), mask)
    assert not torch.allclose(lp1, lp1_other)  # slot b is conditioned on slot a
    value = model.value(state, t["ids"].flip(0), t["floats"].flip(0)[:, 0:6])
    value_other = model.value(state, t["ids"], t["floats"][:, 0:6])
    assert not torch.allclose(value, value_other)  # the critic sees the opponent's team
    aux = model.aux_loss(state, t["floats"], torch.from_numpy(b.foe_match), t["ids"].flip(0))
    assert torch.isfinite(aux)


def test_gae_stops_at_battle_end_and_bootstraps_truncation():
    rewards = np.array([[0.0], [1.0], [0.0]], dtype=np.float32)
    values = np.zeros_like(rewards)
    _, returns = gae(rewards, values, np.array([[False], [True], [False]]), np.array([2.0], dtype=np.float32), 1.0, 1.0)
    np.testing.assert_allclose(returns, [[1.0], [1.0], [2.0]])


def test_minibatches_use_few_sizes_and_no_row_twice():
    from training.ppo import minibatches

    sizes = set()
    for n in range(5000, 7200):
        picks = list(minibatches(np.random.default_rng(n).permutation(n), 1024))
        rows = np.concatenate(picks)
        assert len(np.unique(rows)) == len(rows) and n - len(rows) < 256
        sizes.update(len(p) for p in picks)
    assert sizes == {256, 384, 512, 640, 768, 896, 1024}


def test_adam_matches_torch_adam():
    from training.ppo import Adam

    torch.manual_seed(0)
    ours = torch.randn(5, 3)
    reference = ours.clone().requires_grad_()
    ours.requires_grad_()
    mine, theirs = Adam([ours], lr=1e-2), torch.optim.Adam([reference], lr=1e-2, eps=1e-5, foreach=False)
    for step in range(20):
        grad = torch.randn(5, 3)
        ours.grad, reference.grad = grad.clone(), grad.clone()
        mine.lr = theirs.param_groups[0]["lr"] = 1e-2 * (1 - step / 40)
        mine.step()
        theirs.step()
    torch.testing.assert_close(ours.detach(), reference.detach())


def test_expected_max_and_any_ko_over_possible_moves():
    from training.damage import any_ko, expected_max

    dealt = torch.tensor([0.2, 0.6, 0.4]).view(1, 1, 3, 1)
    probs = torch.tensor([1.0, 0.5, 0.5]).view(1, 1, 3)
    # Sorted: 0.6 (p .5), 0.4 (p .5), 0.2 (p 1): .6*.5 + .4*.5*.5 + .2*1*.25
    assert float(expected_max(dealt, probs)) == pytest.approx(0.3 + 0.1 + 0.05)
    ko = torch.tensor([0.0, 1.0, 0.5]).view(1, 1, 3, 1)
    assert float(any_ko(ko, probs)) == pytest.approx(1 - (1 - 0.5) * (1 - 0.25))
    assert float(expected_max(dealt, torch.zeros(1, 1, 3))) == 0


def fraction(condition: str) -> float:
    """``"123/260 par"`` -> 123 / 260 (own side sees exact HP)."""
    hp = condition.split()[0]
    return 0.0 if hp == "0" else int(hp.split("/")[0]) / int(hp.split("/")[1])


def test_foe_threat_tracks_engine_damage():
    """Per-move damage of foe attacks on own Pokemon (revealed or belief-candidate moves) against the engine."""
    dex = load_dex()
    damage = DamageFeatures(dex)
    env = nb.BatchEnv(16, [str(POOL)], seed=9, threads=2, log=True)
    env.reset()
    encoder = Encoder(32)
    observations = [env.observe(e, s) for e in range(16) for s in (0, 1)]
    ratios, candidates_used = [], 0
    for _ in range(60):
        b = encoder.encode_batch(observations)
        ids, floats, field = (torch.from_numpy(x) for x in (b.ids, b.floats, b.field))
        with torch.no_grad():
            t = damage.tokens(ids, floats, field)
            defenders = {k: v.reshape(len(ids), 1, 1, 12, *v.shape[2:]) for k, v in t.items()}
            dealt, _, probs = damage.foe_attacks(ids, floats, t, defenders, field)
        result = env.step(env.random_actions(switch_prob=0.0, tera_prob=0.0))
        observations = [env.observe(e, s) for e in range(16) for s in (0, 1)]
        for e in range(16):
            row = 2 * e
            if result["done"][e]:
                continue
            lines = observations[row]["log"]
            own = encoder.own_names[row]
            hp_now = {name: float(b.floats[row, j, PF["hp"]]) for j, name in enumerate(own)}  # tracked through the turn
            for i, line in enumerate(lines):
                parts = line.split("|")
                if len(parts) >= 4 and parts[1] in ("-damage", "-heal", "-sethp") and parts[2][:2] == "p1" and ": " in parts[2]:
                    hp_now[parts[2].split(": ", 1)[1]] = fraction(parts[3])
                if len(parts) < 5 or parts[1] != "move" or not parts[2].startswith("p2") or "[" in line:
                    continue
                move = dex.move_index.get(to_id(parts[3]), 0)
                if not move or dex.moves[move - 1]["category"] == "Status" or dex.move_accuracy[move] < 1:
                    continue
                if dex.moves[move - 1]["target"] not in ("normal", "any") or dex.move_special_power[move]:
                    continue
                slot = 0 if parts[2].startswith("p2a") else 1
                options = (list(b.ids[row, 6 + slot, I_MOVE0:I_MOVE0 + 4])
                           + list(b.ids[row, 6 + slot, I_CAND_MOVES:I_CAND_MOVES + K_MOVES]))
                if move not in options or not parts[4].startswith("p1"):
                    continue
                name = parts[4].split(": ", 1)[1]
                if name not in own:
                    continue
                token = own.index(name)
                follow = lines[i + 1:i + 6]
                follow = follow[:next((k for k, f in enumerate(follow) if f.startswith("|move|")), len(follow))]
                hit = [f for f in follow if f.startswith(f"|-damage|{parts[4]}|") and "[from]" not in f]
                if not hit or any(f.startswith(("|-crit", "|-activate", "|-immune")) for f in follow):
                    continue
                now = fraction(hit[0].split("|")[3])
                if now == 0:
                    continue
                actual = hp_now[name] - now  # HP right before this move, not at the start of the turn
                m = options.index(move)
                predicted = float(dealt[row, slot, m, token])
                candidates_used += m >= 4
                if actual > 0.02 and predicted > 0.02:
                    ratios.append(actual / predicted)
    # Measured over 20 seeds: per-seed median 0.98-1.03, quartiles 0.92-0.95 and 1.15-1.40.
    assert len(ratios) > 40 and candidates_used > 0
    q1, median, q3 = np.percentile(ratios, [25, 50, 75])
    assert 0.8 < median < 1.25 and q1 > 0.85 and q3 < 1.5
