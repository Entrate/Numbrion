//! Ports data/items.ts:159 (Aguav Berry). No direct PRNG draws; nested core calls (eatItem,
//! TryHeal, heal, addVolatile) retain event draws.
//!
//! This file owns the batch's private helper module (`consumables/mod.rs`); every other
//! `consumables` file imports it as `crate::effects::registry::items_aguavberry::support`.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
#[path = "consumables/mod.rs"]
pub(super) mod support;
pub const ID: EffectId = dex::ITEM_AGUAVBERRY;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_AGUAVBERRY_ONUPDATE,
    dex::HOOK_ITEM_AGUAVBERRY_ONTRYEATITEM,
    dex::HOOK_ITEM_AGUAVBERRY_ONEAT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:167 onUpdate(pokemon): hp <= maxhp/4, or hp <= maxhp/2 with Gluttony and
        // abilityState.gluttony, then pokemon.eatItem(). Returns undefined. PRNG: none directly.
        dex::HOOK_ITEM_AGUAVBERRY_ONUPDATE => support::pinch_on_update(b, cx),
        // data/items.ts:173 onTryEatItem(item, pokemon): runEvent('TryHeal', ..., baseMaxhp / 3)
        // falsy => false, otherwise undefined. PRNG: none directly.
        dex::HOOK_ITEM_AGUAVBERRY_ONTRYEATITEM => support::pinch_on_try_eat_item(b, cx),
        // data/items.ts:176 onEat(pokemon): heal(baseMaxhp / 3); confusion when the nature's
        // minus stat is spd. Returns undefined. PRNG: none directly.
        dex::HOOK_ITEM_AGUAVBERRY_ONEAT => support::pinch_on_eat(b, cx, support::NATURE_SPD),
        _ => panic!("unexpected Aguav Berry hook"),
    }
}
