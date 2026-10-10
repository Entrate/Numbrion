//! Private helpers for the `consumables` effect batch (berries, Focus Sash, Throat Spray,
//! Weakness Policy, White Herb and the Cheek Pouch / Gluttony / Harvest / Ripen / Unnerve
//! abilities that react to them).
//!
//! Owned by `items/aguavberry.rs` (`pub(super) mod support`); every other file of the batch
//! reaches it through `crate::effects::registry::items_aguavberry::support`.
//! Nothing here allocates, formats a string or draws from the PRNG. Handler bodies stay in
//! their own effect files so each one reads next to its pinned TypeScript.
//!
//! # State bits (EffectCell::present, custom bits 8..30)
//!
//! None of the batch's properties needs a payload word except White Herb's captured
//! boost mask, so every other flag is a single custom presence bit whose *set* state means
//! "JS truthy" (absent, `undefined` and `false` are indistinguishable to every reader):
//!
//! | effect state | property | bit / word |
//! | --- | --- | --- |
//! | Gluttony ability state | `gluttony` | `GLUTTONY_FLAG` (bit 8) |
//! | Ripen ability state | `berryWeaken` | `BERRY_WEAKEN_FLAG` (bit 8) |
//! | Unnerve ability state | `unnerved` | `UNNERVED_FLAG` (bit 8) |
//! | White Herb item state | `boosts` | `HERB_BOOSTS_PRESENT` (bit 8), word 0 = key mask |
#![allow(dead_code)]
use crate::{
    Battle,
    actions::{Attribution, HealEffect},
    dex::{self, EffectType},
    effects::support::optional_id,
    event::{EffectRef, EventArg, HookCtx, Relay, RunEventOptions},
    ids::*,
    log::{LogArg, LogEntry, LogSink},
    state::{Status, present, scratch::OrderedBoosts},
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// `pokemon.abilityState.gluttony` (set by Gluttony's onStart/onDamage, read by the berries).
pub const GLUTTONY_FLAG: u32 = 1 << present::CUSTOM_START;
/// `pokemon.abilityState.berryWeaken` of Ripen.
pub const BERRY_WEAKEN_FLAG: u32 = 1 << present::CUSTOM_START;
/// `this.effectState.unnerved` of Unnerve.
pub const UNNERVED_FLAG: u32 = 1 << present::CUSTOM_START;
/// White Herb `this.effectState.boosts` exists (word 0 holds its keys).
pub const HERB_BOOSTS_PRESENT: u32 = 1 << present::CUSTOM_START;

/// Nature stat indices (`NatureData::plus/minus`): 0 hp, 1 atk, 2 def, 3 spa, 4 spd, 5 spe;
/// -1 for the five neutral natures (JS `undefined`), which never equal a stat key.
pub const NATURE_ATK: i8 = 1;
pub const NATURE_DEF: i8 = 2;
pub const NATURE_SPA: i8 = 3;
pub const NATURE_SPD: i8 = 4;
pub const NATURE_SPE: i8 = 5;

/// Required Pokemon callback argument.
pub fn mon<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> MonId {
    Battle::<L>::arg_mon(b.event_arg(cx, index)).expect("callback requires Pokemon argument")
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

/// `this.effectState.target` as a Pokemon (the holder of the executing item/ability).
pub fn state_target<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> MonId {
    let holder = b.hook_state(cx).target;
    assert!(holder.0 < 12, "effect state target is not a Pokemon");
    MonId(holder.0)
}

/// `effect.effectType === 'Move'` with an absent effect (`effect &&`) counting as false.
pub fn effect_is_move<L: LogSink>(b: &Battle<L>, effect: EffectRef) -> bool {
    effect != EffectRef::None && b.event_effect_type(effect) == EffectType::Move
}

/// `item.isBerry` for an effect that may or may not be an item (`(effect as Item).isBerry`).
pub fn effect_is_berry<L: LogSink>(b: &Battle<L>, effect: EffectRef) -> bool {
    item_is_berry(b.event_effect_id(effect))
}

/// `this.dex.items.get(id).isBerry`; the empty id and every non-item id are not berries.
pub fn item_is_berry(id: EffectId) -> bool {
    id.kind() == Some(EffectKind::Item) && dex::ITEMS[(id.0 - dex::ITEM_START) as usize].berry
}

/// `effect.name === 'X'` for a single scoped id; a name missing from the scoped dex
/// resolves to `EffectId::NONE`, which must never match (NONE is also the id of absent effects).
pub fn is_named(id: EffectId, wanted: EffectId) -> bool {
    id != EffectId::NONE && id == wanted
}

/// `list.includes(item.name)` over a table of optional ids (see `is_named`).
pub fn is_listed(id: EffectId, list: &[EffectId]) -> bool {
    id != EffectId::NONE && list.contains(&id)
}

/// Compile-time `optional_id` over the scoped item table.
pub const fn item_named(key: &str) -> EffectId {
    optional_id(dex::ITEMS_DATA, key)
}

// ---------------------------------------------------------------------------------------
// Berries
// ---------------------------------------------------------------------------------------

/// `pokemon.abilityState.gluttony` truthiness (pokemon.abilityState is read as the raw cell).
pub fn gluttony_flag<L: LogSink>(b: &Battle<L>, pokemon: MonId) -> bool {
    let cell = b.state.pokemon[pokemon.0 as usize].ability_state;
    b.state.effects.cells[cell.0 as usize].present & GLUTTONY_FLAG != 0
}

/// `pokemon.abilityState.gluttony = true;`
pub fn set_gluttony<L: LogSink>(b: &mut Battle<L>, pokemon: MonId) {
    let cell = b.state.pokemon[pokemon.0 as usize].ability_state;
    b.state.effects.cells[cell.0 as usize].present |= GLUTTONY_FLAG;
}

/// `pokemon.hp <= pokemon.maxhp / divisor` (JS float division, no rounding).
pub fn hp_at_most_fraction<L: LogSink>(b: &Battle<L>, pokemon: MonId, divisor: f64) -> bool {
    let p = &b.state.pokemon[pokemon.0 as usize];
    f64::from(p.hp) <= f64::from(p.max_hp) / divisor
}

/// The shared `onUpdate` condition of Aguav/Figy/Iapapa/Mago/Wiki:
/// `pokemon.hp <= pokemon.maxhp / 4 || (pokemon.hp <= pokemon.maxhp / 2 &&
///  pokemon.hasAbility('gluttony') && pokemon.abilityState.gluttony)`.
pub fn pinch_berry_ready<L: LogSink>(b: &Battle<L>, pokemon: MonId) -> bool {
    hp_at_most_fraction(b, pokemon, 4.0)
        || (hp_at_most_fraction(b, pokemon, 2.0)
            && b.has_ability(pokemon, &[dex::ABILITY_GLUTTONY])
            && gluttony_flag(b, pokemon))
}

/// `pokemon.eatItem();` (force/source/sourceEffect omitted).
pub fn eat_item<L: LogSink>(b: &mut Battle<L>, pokemon: MonId) {
    b.eat_item(pokemon, Attribution::DEFAULT, false);
}

/// `onTryEatItem(item, pokemon)` of the healing berries:
/// `if (!this.runEvent('TryHeal', pokemon, null, this.effect, pokemon.baseMaxhp / divisor)) return false;`
/// baseMaxhp equals maxhp in this format (no Dynamax). Returns `false` or `undefined`.
pub fn try_eat_heal<L: LogSink>(b: &mut Battle<L>, pokemon: MonId, divisor: f64) -> Relay {
    let amount = f64::from(b.state.pokemon[pokemon.0 as usize].max_hp) / divisor;
    let effect = b.scratch.current_effect;
    let result = b.run_event(
        dex::EventId::TryHeal,
        EventArg::Holder(Holder::mon(pokemon)),
        EventArg::Null,
        effect,
        Relay::Number(amount),
        RunEventOptions::default(),
    );
    if !result.truthy() {
        return Relay::Bool(false);
    }
    Relay::Undefined
}

/// `this.heal(pokemon.baseMaxhp / divisor);` target/source/effect all default from the event.
pub fn heal_base_max_hp<L: LogSink>(b: &mut Battle<L>, pokemon: MonId, divisor: f64) {
    let amount = f64::from(b.state.pokemon[pokemon.0 as usize].max_hp) / divisor;
    b.heal(amount, None, None, HealEffect::Context);
}

/// `if (pokemon.getNature().minus === 'stat') pokemon.addVolatile('confusion');`
pub fn confuse_if_disliked<L: LogSink>(b: &mut Battle<L>, pokemon: MonId, stat: i8) {
    // getNature() of a set without a nature is the empty nature: `minus` is undefined.
    let minus = b.get_nature(pokemon).map(|n| n.minus);
    if minus == Some(stat) {
        b.add_volatile(
            pokemon,
            dex::CONDITION_CONFUSION,
            Attribution::DEFAULT,
            None,
        );
    }
}

/// `pokemon.status === 'slp'`
pub fn is_asleep<L: LogSink>(b: &Battle<L>, pokemon: MonId) -> bool {
    b.state.pokemon[pokemon.0 as usize].status == Status::Sleep
}

// ---------------------------------------------------------------------------------------
// White Herb
// ---------------------------------------------------------------------------------------

/// Keys of `pokemon.boosts` with a negative stage, as a bit per `boosts` index. The JS object is
/// created in the order atk, def, spa, spd, spe, accuracy, evasion, so a `for (i in boosts)`
/// walk is exactly ascending index order and the mask loses nothing.
pub fn negative_boost_mask<L: LogSink>(b: &Battle<L>, pokemon: MonId) -> u8 {
    let boosts = &b.state.pokemon[pokemon.0 as usize].boosts;
    let mut mask = 0;
    for (i, stage) in boosts.iter().enumerate() {
        if *stage < 0 {
            mask |= 1 << i;
        }
    }
    mask
}

/// `{ atk: 0, ... }` for every key in `mask` (a SparseBoostsTable with present zero values).
pub fn zeroed_boosts(mask: u8) -> OrderedBoosts {
    let mut out = OrderedBoosts::default();
    for i in 0..7u8 {
        if mask & (1 << i) != 0 {
            out.order[out.len as usize] = i;
            out.len += 1;
            out.present |= 1 << i;
        }
    }
    out
}

/// `this.add('-clearnegativeboost', pokemon, '[silent]');`
pub fn add_clear_negative_boost<L: LogSink>(b: &mut Battle<L>, pokemon: MonId) {
    b.add(LogEntry::new(
        "-clearnegativeboost",
        &[LogArg::Mon(pokemon)],
        &[crate::log::LogTag::Bare("silent")],
    ));
}

/// The shared body of White Herb's `fling.effect`. Kept here so tests can reach it.
pub fn clear_negative_boosts_now<L: LogSink>(b: &mut Battle<L>, pokemon: MonId) {
    let mask = negative_boost_mask(b, pokemon);
    // if (activate) { pokemon.setBoost(boosts); this.add('-clearnegativeboost', ...); }
    if mask != 0 {
        b.set_boost(pokemon, zeroed_boosts(mask));
        add_clear_negative_boost(b, pokemon);
    }
}
