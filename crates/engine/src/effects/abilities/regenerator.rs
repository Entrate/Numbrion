//! Ports data/abilities.ts:3834 (Regenerator). No PRNG draws; `Pokemon.heal` is the raw HP
//! restore (no TryHeal/Heal events, no protocol line).
use crate::effects::registry::abilities_baddreams::support::base_max_hp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_REGENERATOR;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_REGENERATOR_ONSWITCHOUT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_REGENERATOR_ONSWITCHOUT => on_switch_out(b, cx),
        _ => panic!("unexpected regenerator hook"),
    }
}

// data/abilities.ts:3835-3837 onSwitchOut(pokemon). PRNG: none.
fn on_switch_out<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // pokemon.heal(pokemon.baseMaxhp / 3);  (Pokemon.heal truncates the fraction itself)
    let amount = base_max_hp(b, pokemon) / 3.0;
    b.raw_heal(pokemon, amount);
    Relay::Undefined
}
