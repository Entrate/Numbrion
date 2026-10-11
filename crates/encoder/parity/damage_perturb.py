"""Edge-case inputs for the damage-feature parity check (examples/parity_damage.rs).

Takes the ``ids`` / ``floats`` / ``field`` of a reference array directory (reference.py) and makes ``--copies``
perturbed copies: each row independently gets a random subset of Tera availability and Tera types (Stellar included),
terastallized tokens, Trick Room, Gravity, terrains, weather, screens, Tailwind, burn and paralysis, boosts, zero-HP
and absent tokens, speed ties, the abilities, items and moves damage.py special-cases (Prankster, Gale Wings, Mold
Breaker, Sheer Force, Unseen Fist, Covert Cloak, Fake Out, Tera Blast, Grassy Glide, Protect, flinch, spread,
variable-power and priority moves, ...), unknown abilities and items with beliefs, and foes without any move
probability (the generic STAB attacks). Saves those inputs with torch ``DamageFeatures`` outputs
(``action_features``, ``token_features``) as a new raw array directory.

Usage: python damage_perturb.py REF_DIR OUT_DIR [--copies 3] [--seed 0] [--time]
"""
from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[1] / "pyengine"))

import numpy as np  # noqa: E402
import torch  # noqa: E402

import arrays  # noqa: E402
from training.damage import DamageFeatures  # noqa: E402
from training.dex import load_dex  # noqa: E402
from training.encoder import (FF, I_ABILITY, I_CAND_ABILITIES, I_CAND_ITEMS, I_CAND_MOVES, I_ITEM, I_MOVE0,  # noqa: E402
                              I_SPECIES, I_TERA, I_TYPE1, K_ABILITIES, K_ITEMS, K_MOVES, PF)

STELLAR = 19
ABILITIES = ("prankster", "galewings", "sheerforce", "serenegrace", "moldbreaker", "teravolt", "turboblaze",
             "unseenfist", "hugepower", "purepower", "innerfocus", "shielddust", "levitate", "armortail", "dazzling",
             "queenlymajesty", "flashfire", "waterabsorb", "dryskin", "thickfat")
ITEMS = ("choicescarf", "choiceband", "choicespecs", "lifeorb", "covertcloak", "airballoon", "leftovers")
MOVES = ("fakeout", "firstimpression", "terablast", "grassyglide", "protect", "detect", "spikyshield", "triplearrows",
         "airslash", "ironhead", "rockslide", "nightshade", "superfang", "ruination", "lowkick", "grassknot",
         "heavyslam", "heatcrash", "eruption", "waterspout", "dragonenergy", "endeavor", "finalgambit", "earthquake",
         "surf", "heatwave", "dazzlinggleam", "outrage", "petaldance", "thunderwave", "willowisp", "bravebird",
         "extremespeed", "suckerpunch", "trickroom", "shadowball", "flamethrower", "hydropump", "bulletseed",
         "tripleaxel", "uturn", "closecombat", "machpunch", "aquajet", "drainpunch", "zenheadbutt", "waterfall")
FIELD_FLAGS = ("trickroom", "gravity", "psychicterrain", "grassyterrain", "sun", "rain", "own_reflect",
               "own_lightscreen", "own_auroraveil", "foe_reflect", "foe_lightscreen", "foe_auroraveil", "own_tailwind",
               "foe_tailwind")
BOOSTS = ("boost_atk", "boost_def", "boost_spa", "boost_spd", "boost_spe")


def perturb(ids: np.ndarray, floats: np.ndarray, field: np.ndarray, rng: np.random.Generator, dex) -> None:
    """Perturb [N, 12, ...] rows in place."""
    n = len(ids)
    abilities = [dex.ability_index[a] for a in ABILITIES if a in dex.ability_index]
    items = [dex.item_index[i] for i in ITEMS if i in dex.item_index]
    moves = [dex.move_index[m] for m in MOVES if m in dex.move_index]
    chance = lambda p, *shape: rng.random(shape or (n,)) < p  # noqa: E731
    pick = lambda pool, *shape: np.asarray(pool)[rng.integers(0, len(pool), shape or (n,))]  # noqa: E731
    actives = [0, 1, 6, 7]

    # Tera: own actives can terastallize (often into Stellar), random tokens already are.
    for s in (0, 1):
        rows = chance(0.5)
        floats[rows, s, PF["tera_available"]] = 1.0
        floats[rows, s, PF["terastallized"]] = 0.0
        rows = chance(0.35)
        ids[rows, s, I_TERA] = np.where(chance(0.4)[rows], STELLAR, rng.integers(0, 20, rows.sum()))
    for i in range(12):
        rows = chance(0.15)
        floats[rows, i, PF["terastallized"]] = 1.0
        ids[rows, i, I_TERA] = np.where(chance(0.25)[rows], STELLAR, rng.integers(0, 19, rows.sum()))
    # Field conditions.
    for name in FIELD_FLAGS:
        field[chance(0.15), FF[name]] = 1.0
    # Statuses, boosts, HP, presence, Protect and Fake Out state.
    for i in range(12):
        floats[chance(0.1), i, PF["brn"]] = 1.0
        floats[chance(0.1), i, PF["par"]] = 1.0
        for b in BOOSTS:
            rows = chance(0.15)
            floats[rows, i, PF[b]] = (rng.integers(-6, 7, rows.sum()) / 6).astype(np.float32)
        rows = chance(0.08)
        floats[rows, i, PF["hp"]] = 0.0
        floats[chance(0.04), i, PF["fainted"]] = 1.0
        floats[chance(0.15), i, PF["hp"]] = 1.0
        floats[chance(0.04), i, PF["present"]] = 0.0
        floats[chance(0.2), i, PF["fresh"]] = 1.0
        floats[chance(0.2), i, PF["protect_last"]] = 1.0
        floats[chance(0.1), i, PF["substitute"]] = 1.0
        rows = chance(0.1)
        floats[rows, i, PF["type_mult0"] + rng.integers(0, 20, rows.sum())] = pick((0.0, 0.5, 1.25, 0.3), rows.sum())
        rows = chance(0.1)
        ids[rows, i, I_SPECIES] = rng.integers(1, dex.n_species, rows.sum())
    # Speed ties between an own and a foe active (same stat, boost, paralysis, item and Tailwind).
    rows = np.flatnonzero(chance(0.25))
    s, f = pick((0, 1), len(rows)), pick((6, 7), len(rows))
    for col in ("stat_spe", "boost_spe", "par"):
        floats[rows, f, PF[col]] = floats[rows, s, PF[col]]
    ids[rows, f, I_ITEM] = ids[rows, s, I_ITEM]
    field[rows, FF["foe_tailwind"]] = field[rows, FF["own_tailwind"]]
    # Abilities and items: known special ones, or unknown with candidates.
    for i in actives + [2, 8]:
        rows = chance(0.3)
        ids[rows, i, I_ABILITY] = pick(abilities, rows.sum())
        rows = chance(0.15)
        ids[rows, i, I_ABILITY] = 0
        ids[rows, i, I_CAND_ABILITIES:I_CAND_ABILITIES + K_ABILITIES] = pick(abilities, rows.sum(), K_ABILITIES)
        floats[rows, i, PF["cand_ability_p0"]:PF["cand_ability_p0"] + K_ABILITIES] = rng.random((rows.sum(), K_ABILITIES)) * 0.6
        rows = chance(0.3)
        ids[rows, i, I_ITEM] = pick(items, rows.sum())
        rows = chance(0.15)
        ids[rows, i, I_ITEM] = 0
        ids[rows, i, I_CAND_ITEMS:I_CAND_ITEMS + K_ITEMS] = pick(items, rows.sum(), K_ITEMS)
        floats[rows, i, PF["cand_item_p0"]:PF["cand_item_p0"] + K_ITEMS] = rng.random((rows.sum(), K_ITEMS)) * 0.6
    # Moves: own and foe actives get special-cased moves; foes revealed or believed; some foes have no move beliefs.
    for i in actives:
        for k in range(4):
            rows = chance(0.3)
            ids[rows, i, I_MOVE0 + k] = pick(moves, rows.sum())
            floats[rows, i, PF["move_known0"] + k] = 1.0
        rows = chance(0.3)
        ids[rows, i, I_CAND_MOVES:I_CAND_MOVES + K_MOVES] = pick(moves, rows.sum(), K_MOVES)
        floats[rows, i, PF["cand_move_p0"]:PF["cand_move_p0"] + K_MOVES] = rng.random((rows.sum(), K_MOVES)) * 0.7
    for f in (6, 7):
        rows = chance(0.1)
        floats[rows, f, PF["move_known0"]:PF["move_known0"] + 4] = 0.0
        floats[rows, f, PF["cand_move_p0"]:PF["cand_move_p0"] + K_MOVES] = 0.0
        rows = chance(0.05)
        ids[rows, f, I_MOVE0:I_MOVE0 + 4] = 0
        ids[rows, f, I_CAND_MOVES:I_CAND_MOVES + K_MOVES] = 0
        rows = chance(0.1)
        ids[rows, f, I_TYPE1 + 1] = 0  # mono-type foe: a generic attack of type 0 deals nothing


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("reference", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--copies", type=int, default=3)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--time", action="store_true", help="time DamageFeatures on the unperturbed rows, batch 16")
    args = parser.parse_args()
    ref = arrays.load(args.reference)
    T, R = ref["ids"].shape[:2]
    dex = load_dex()
    damage = DamageFeatures(dex)
    torch.set_num_threads(1)
    if args.time:
        ids_t = torch.from_numpy(ref["ids"].reshape(T * R, 12, -1))
        floats_t = torch.from_numpy(ref["floats"].reshape(T * R, 12, -1))
        field_t = torch.from_numpy(ref["field"].reshape(T * R, -1))
        with torch.no_grad():
            started = time.perf_counter()
            for b in range(0, T * R, 16):
                damage(ids_t[b:b + 16], floats_t[b:b + 16], field_t[b:b + 16])
            seconds = time.perf_counter() - started
        print(f"torch DamageFeatures: {T * R} rows in {seconds:.2f}s ({1e3 * seconds / (T * R):.3f} ms/row, batch 16)")

    rng = np.random.default_rng(args.seed)
    out = {k: [] for k in ("ids", "floats", "field", "action_features", "token_features")}
    for _ in range(args.copies):
        ids = ref["ids"].reshape(T * R, 12, -1).copy()
        floats = ref["floats"].reshape(T * R, 12, -1).copy()
        field = ref["field"].reshape(T * R, -1).copy()
        perturb(ids, floats, field, rng, dex)
        with torch.no_grad():
            action, token = damage(torch.from_numpy(ids), torch.from_numpy(floats), torch.from_numpy(field))
        for k, v in (("ids", ids), ("floats", floats), ("field", field), ("action_features", action.numpy()),
                     ("token_features", token.numpy())):
            out[k].append(v.reshape(T, R, *v.shape[1:]))
    arrays.save(args.out, {k: np.concatenate(v) for k, v in out.items()})
    print(f"{args.copies} x {T} x {R} perturbed rows -> {args.out}")


if __name__ == "__main__":
    main()
