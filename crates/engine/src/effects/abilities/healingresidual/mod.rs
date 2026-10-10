//! Private helpers for the `healing_residual` effect batch (residual HP, cures, passive
//! recovery and draining conditions across moves, abilities, items and conditions).
//!
//! Owned by `baddreams.rs` (`pub(super) mod support`); the other files of the batch reach it
//! through `crate::effects::registry::abilities_baddreams::support`. Nothing here allocates,
//! formats a string or draws from the PRNG: the handler bodies stay in their own effect files
//! so each one reads next to its pinned TypeScript.
#![allow(dead_code)]
use crate::{
    Battle,
    actions::{HitTarget, Stat},
    dex,
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::LogSink,
    state::scratch::OrderedBoosts,
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Compile-time lookup of a type by display name (`move.type === 'Water'`). Mirrors
/// `dex::type_id`, so ids stay in lockstep with the generated table.
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
pub const TYPE_FIRE: TypeId = type_named("Fire");
pub const TYPE_GRASS: TypeId = type_named("Grass");
pub const TYPE_STEEL: TypeId = type_named("Steel");
pub const TYPE_WATER: TypeId = type_named("Water");

/// A JS boost object literal such as `{ spa: 1, spd: 1 }`: insertion order is the argument
/// order and every key is distinct at all call sites (Object.keys order drives `boost()`).
pub fn boosts<const N: usize>(entries: [(Stat, i8); N]) -> OrderedBoosts {
    let mut out = OrderedBoosts::default();
    for (stat, amount) in entries {
        let i = stat as usize;
        debug_assert_eq!(out.present & (1 << i), 0, "duplicate boost key");
        out.values[i] = amount;
        out.order[out.len as usize] = i as u8;
        out.len += 1;
        out.present |= 1 << i;
    }
    out
}

/// `pokemon.baseMaxhp`. Dynamax is outside this format, so `baseMaxhp === maxhp`
/// (pokemon.ts:1498-1506 is the only place they diverge; see `Battle::get_undynamaxed_hp`).
pub fn base_max_hp<L: LogSink>(b: &Battle<L>, mon: MonId) -> f64 {
    f64::from(b.state.pokemon[mon.0 as usize].max_hp)
}

/// Nullable Pokemon callback argument (`source` may be null or undefined).
pub fn mon_opt<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> Option<MonId> {
    Battle::<L>::arg_mon(b.event_arg(cx, index))
}

/// Effect-like callback argument as an `EffectRef` (`None` for null or undefined).
pub fn effect_at<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> EffectRef {
    match b.event_arg(cx, index) {
        EventArg::Effect(e) => e,
        EventArg::Move(m) | EventArg::Relay(Relay::ActiveMove(m)) => EffectRef::ActiveMove(m),
        EventArg::Relay(Relay::Effect(id)) | EventArg::Relay(Relay::Move(id)) => EffectRef::Dex(id),
        EventArg::Undefined
        | EventArg::Null
        | EventArg::Relay(Relay::Undefined)
        | EventArg::Relay(Relay::Null) => EffectRef::None,
        _ => panic!("callback requires effect argument"),
    }
}

/// The entries of `pokemon.foes()` / `adjacentAllies()` are always Pokemon.
pub fn listed_mon(entry: HitTarget) -> MonId {
    match entry {
        HitTarget::Pokemon(m) => m,
        _ => panic!("Pokemon list contains a non-Pokemon entry"),
    }
}

/// JS numeric view of a truthy `damage` relay (`heal(damage, ...)` receives the number the
/// previous call returned; `true` would be coerced to 1 by `damage <= 1`).
pub fn relay_amount(relay: Relay) -> f64 {
    match relay {
        Relay::Number(n) => n,
        Relay::Bool(true) => 1.0,
        other => panic!("damage relay {other:?} is not a heal amount"),
    }
}
