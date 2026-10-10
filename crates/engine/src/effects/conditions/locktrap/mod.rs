//! Batch-private helpers for the `disable_lock_trap` effect files (Disable/Encore/Taunt/locks/trapping).
//!
//! Declared once in `conditions/choicelock.rs` (`#[path = "locktrap/mod.rs"] pub(super) mod support;`) and used
//! by the sibling files through `crate::effects::registry::conditions_choicelock::support`. Discovery ignores
//! nested directories. Nothing here draws from the PRNG, allocates, or raises an event; every helper is a
//! straight read/write of Battle state, so each handler keeps its source statement order explicit.
//!
//! Payload conventions shared by Choice lock, Disable and Encore: `payload.words[0]` holds the stored
//! `effectState.move` as a raw `EffectId`, guarded by the custom present bit [`MOVE_PRESENT`] (bit 8).
use crate::{
    Battle, dex,
    event::{EffectRef, HookCtx},
    ids::*,
    log::LogSink,
    state::{mon_flags, present},
};

/// Type ids are the 1-based index into `dex::TYPE_NAMES` ("Ghost" = 8, "Steel" = 9; 0 is the empty type).
pub const TYPE_GHOST: TypeId = TypeId(8);
pub const TYPE_STEEL: TypeId = TypeId(9);

/// `effectState.move` present bit (first custom bit); the move id itself is `payload.words[0]`.
pub const MOVE_PRESENT: u32 = 1 << present::CUSTOM_START;

/// `source?.`-style read of a Pokemon-valued callback argument: undefined/null become `None`.
pub fn opt_mon_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> Option<MonId> {
    Battle::<L>::arg_mon(b.event_arg(cx, index))
}

/// `this.effectState.move`: `EffectId::NONE` while absent (the TS value is then `undefined`).
pub fn stored_move<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> EffectId {
    let c = b.hook_state(cx);
    if c.present & MOVE_PRESENT == 0 {
        EffectId::NONE
    } else {
        EffectId(c.payload.words[0] as u16)
    }
}

/// `this.effectState.move = id`.
pub fn store_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, id: EffectId) {
    let c = b.hook_state_mut(cx);
    c.payload.words[0] = u32::from(id.0);
    c.present |= MOVE_PRESENT;
}

/// `this.effectState.source` (set by addVolatile through `condition_source`); `None` while absent.
pub fn state_source<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> Option<MonId> {
    let c = b.hook_state(cx);
    (c.present & present::SOURCE != 0 && c.source != MonId::NONE).then_some(c.source)
}

/// `this.effectState.sourceEffect`: the frozen effect token written by addVolatile, else `None`.
pub fn state_source_effect<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> EffectRef {
    let c = b.hook_state(cx);
    if c.present & present::SOURCE_EFFECT == 0 {
        EffectRef::None
    } else {
        EffectToken(c.source_effect.0).resolve()
    }
}

/// `this.effectState.target` of an ability/condition cell as a Pokemon (`None` for singleEvent's `{}`).
pub fn state_target<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> Option<MonId> {
    let c = b.hook_state(cx);
    (c.present & present::TARGET != 0 && c.target.0 < 12).then_some(MonId(c.target.0))
}

/// Snapshot of `pokemon.moveSlots[*].id` so `disableMove` can be called between reads.
pub fn move_slot_ids<L: LogSink>(b: &Battle<L>, m: MonId) -> ([EffectId; 4], usize) {
    let slots = b.state.pokemon[m.0 as usize].move_slots();
    let mut ids = [EffectId::NONE; 4];
    for (i, slot) in slots.iter().enumerate() {
        ids[i] = slot.id;
    }
    (ids, slots.len())
}

/// `pokemon.getItem().isChoice` (the item object, regardless of Klutz/Embargo; empty item is not a Choice).
pub fn holds_choice_item<L: LogSink>(b: &Battle<L>, m: MonId) -> bool {
    let item = b.state.pokemon[m.0 as usize].item;
    item != EffectId::NONE && dex::ITEMS[(item.0 - dex::ITEM_START) as usize].choice
}

/// `pokemon.maybeDisabled = true` / `maybeTrapped = true` request hypotheses live in `Pokemon.flags`.
pub fn set_flag<L: LogSink>(b: &mut Battle<L>, m: MonId, flag: u32) {
    b.state.pokemon[m.0 as usize].flags |= flag;
}

pub const MAYBE_TRAPPED: u32 = mon_flags::MAYBE_TRAPPED;
pub const MAYBE_DISABLED: u32 = mon_flags::MAYBE_DISABLED;

/// `for (const moveSlot of pokemon.moveSlots) if (pred(moveSlot.id)) pokemon.disableMove(moveSlot.id)`.
/// `disableMove(id)` without a source effect defaults to the executing effect inside the core mutator.
pub fn disable_matching<L: LogSink>(
    b: &mut Battle<L>,
    m: MonId,
    mut pred: impl FnMut(&Battle<L>, EffectId) -> bool,
) {
    let (ids, len) = move_slot_ids(b, m);
    for &id in &ids[..len] {
        if pred(b, id) {
            b.disable_move(m, id, false, EffectRef::None);
        }
    }
}

#[cfg(test)]
mod tests;
