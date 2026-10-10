//! Ports data/abilities.ts:1875 (Hospitality). No direct PRNG draws; each `heal` runs
//! TryHeal/Heal, whose listeners may speed-sort ties.
//!
//! `onSwitchInPriority: -2` is manifest metadata (absent site HOOK_ABILITY_HOSPITALITY_ONSWITCHIN);
//! the callback itself is `onStart`, reached through the SwitchIn -> onStart fallback.
use crate::effects::registry::abilities_baddreams::support::{base_max_hp, listed_mon};
use crate::{
    Battle,
    actions::HealEffect,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_HOSPITALITY;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_HOSPITALITY_ONSTART];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_HOSPITALITY_ONSTART => on_start(b, cx),
        _ => panic!("unexpected hospitality hook"),
    }
}

// data/abilities.ts:1877-1881 onStart(pokemon). PRNG: none directly (heal events only).
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // for (const ally of pokemon.adjacentAllies()): captured once, before any heal.
    let allies = b.adjacent_allies(pokemon);
    for entry in &allies.entries[..allies.len as usize] {
        let ally = listed_mon(*entry);
        // this.heal(ally.baseMaxhp / 4, ally, pokemon);  effect defaults to this.effect
        let amount = base_max_hp(b, ally) / 4.0;
        b.heal(amount, Some(ally), Some(pokemon), HealEffect::Context);
    }
    Relay::Undefined
}
