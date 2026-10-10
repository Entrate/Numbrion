//! Ports data/moves.ts:117-136 (Acrobatics). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
#[path = "movecallbacks/mod.rs"]
mod support;
// The whole batch's scenario replays and helper tests live in one module, compiled here only.
#[cfg(test)]
#[path = "movecallbacks/tests.rs"]
mod scenario_tests;
pub const ID: EffectId = dex::MOVE_ACROBATICS;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_ACROBATICS_BASEPOWERCALLBACK];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_ACROBATICS_BASEPOWERCALLBACK => base_power_callback(b, cx),
        _ => panic!("unexpected Acrobatics function site"),
    }
}

// data/moves.ts:121-127 basePowerCallback(pokemon, target, move): direct call from
// getDamage (battle-actions.ts:1605) with args [pokemon, target, move]. `!pokemon.item` is the
// empty item id; `move.basePower` is the live ActiveMove number. `this.debug` prints nothing.
// PRNG: none.
fn base_power_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let base_power = support::live_base_power(b, cx, 2);
    if b.state.pokemon[pokemon.0 as usize].item == EffectId::NONE {
        return Relay::Number(base_power * 2.0);
    }
    Relay::Number(base_power)
}
