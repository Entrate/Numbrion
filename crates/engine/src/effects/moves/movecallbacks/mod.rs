//! Private helpers for the `move_callbacks` effect batch (variable base power, damage
//! callbacks and small move-specific hit effects). Nothing here allocates, formats a
//! string or draws from the PRNG; handler bodies stay in their own effect files so each one
//! can be read next to its pinned TypeScript.
//!
//! Every effect file that needs these helpers declares
//! `#[path = "movecallbacks/mod.rs"] mod support;` (the `itemsmodifiers` pattern); the module
//! is therefore compiled once per user and carries no state.
#![allow(dead_code)]
use crate::{
    Battle,
    actions::MoveHandle,
    dex,
    effects::support::{mon_arg, move_arg},
    event::{EventArg, HookCtx, Relay},
    ids::*,
    log::LogSink,
};

/// Compile-time lookup of a type by its display name (`move.type === 'Dark'`).
/// Mirrors `dex::type_id`, so ids stay in lockstep with the generated table.
pub const fn type_named(name: &str) -> TypeId {
    let want = name.as_bytes();
    let mut i = 0;
    while i < dex::TYPE_NAMES.len() {
        let have = dex::TYPE_NAMES[i].as_bytes();
        if have.len() == want.len() {
            let mut j = 0;
            let mut same = true;
            while j < have.len() {
                if have[j] != want[j] {
                    same = false;
                }
                j += 1;
            }
            if same {
                return TypeId(i as u8 + 1);
            }
        }
        i += 1;
    }
    panic!("unknown type name")
}
pub const TYPE_ELECTRIC: TypeId = type_named("Electric");
pub const TYPE_WATER: TypeId = type_named("Water");
pub const TYPE_QUESTION: TypeId = type_named("???");

/// `Pokemon | undefined` argument of a callback (a missing target stays `None`).
pub fn opt_mon_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> Option<MonId> {
    Battle::<L>::arg_mon(b.event_arg(cx, index))
}

/// The live `move.basePower` of the ActiveMove argument (a mutable JS property, so the
/// overlay value rather than the immutable dex number).
pub fn live_base_power<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> f64 {
    b.active_move(MoveHandle(move_arg(b, cx, index))).base_power
}

/// `move.basePower * pokemon.hp / pokemon.maxhp` for basePowerCallback(pokemon, target, move)
/// (Eruption, Water Spout, Dragon Energy; data/moves.ts:4137,4892,20665). JS evaluates
/// `(basePower * hp) / maxhp` left to right and the caller floors the fraction, so keep f64.
pub fn hp_scaled_base_power<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let base_power = live_base_power(b, cx, 2);
    let p = &b.state.pokemon[pokemon.0 as usize];
    Relay::Number(base_power * f64::from(p.hp) / f64::from(p.max_hp))
}

/// Grass Knot / Low Kick weight table (data/moves.ts:7544-7556 and 10449-10461).
pub fn target_weight_base_power(target_weight: f64) -> f64 {
    if target_weight >= 2000.0 {
        120.0
    } else if target_weight >= 1000.0 {
        100.0
    } else if target_weight >= 500.0 {
        80.0
    } else if target_weight >= 250.0 {
        60.0
    } else if target_weight >= 100.0 {
        40.0
    } else {
        20.0
    }
}

/// Heat Crash / Heavy Slam weight-ratio table (data/moves.ts:8484-8494 and 8541-8551).
/// Comparisons keep the JS operand form `pokemonWeight >= targetWeight * k` (NaN falls to 40).
pub fn weight_ratio_base_power(pokemon_weight: f64, target_weight: f64) -> f64 {
    if pokemon_weight >= target_weight * 5.0 {
        120.0
    } else if pokemon_weight >= target_weight * 4.0 {
        100.0
    } else if pokemon_weight >= target_weight * 3.0 {
        80.0
    } else if pokemon_weight >= target_weight * 2.0 {
        60.0
    } else {
        40.0
    }
}

/// `this.clampIntRange(Math.floor(hp / 2), 1)` / `clampIntRange(hp / 2, 1)` (the helper floors
/// first, lib/utils.ts:320) for `target.getUndynamaxedHP()` (Ruination, Super Fang;
/// data/moves.ts:15532,18466). No Dynamax in this format, so the HP is the current HP.
pub fn half_target_hp_damage<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 1);
    let hp = b.get_undynamaxed_hp(target, None);
    Relay::Number((hp / 2.0).floor().max(1.0))
}

/// Pokemon argument as the event argument form accepted by mutators.
pub fn mon_event_arg(m: MonId) -> EventArg {
    EventArg::Holder(Holder::mon(m))
}
