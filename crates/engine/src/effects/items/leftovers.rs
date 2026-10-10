//! Ports data/items.ts:3338 (Leftovers). No direct PRNG draws; the `heal` runs TryHeal/Heal
//! events (Ripen/Heal Block style listeners) whose listeners may speed-sort ties.
use crate::effects::registry::abilities_baddreams::support::base_max_hp;
use crate::{
    Battle,
    actions::HealEffect,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ITEM_LEFTOVERS;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_LEFTOVERS_ONRESIDUAL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_LEFTOVERS_ONRESIDUAL => on_residual(b, cx),
        _ => panic!("unexpected leftovers hook"),
    }
}

// data/items.ts:3346-3348 onResidual(pokemon). PRNG: none directly.
// Residual order 5 / subOrder 4 is manifest metadata.
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // this.heal(pokemon.baseMaxhp / 16);  target/source/effect default from the Residual event
    let amount = base_max_hp(b, pokemon) / 16.0;
    b.heal(amount, None, None, HealEffect::Context);
    Relay::Undefined
}
