//! Ports data/moves.ts:4888-4905 (Eruption). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
#[path = "movecallbacks/mod.rs"]
mod support;
pub const ID: EffectId = dex::MOVE_ERUPTION;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_ERUPTION_BASEPOWERCALLBACK];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_ERUPTION_BASEPOWERCALLBACK => base_power_callback(b, cx),
        _ => panic!("unexpected Eruption function site"),
    }
}

// data/moves.ts:4892-4896 basePowerCallback(pokemon, target, move): direct call, args
// [pokemon, target, move]. `move.basePower * pokemon.hp / pokemon.maxhp` is returned as the
// unrounded JS number; getDamage floors/clamps it (battle-actions.ts:1604-1609). PRNG: none.
fn base_power_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    support::hp_scaled_base_power(b, cx)
}
