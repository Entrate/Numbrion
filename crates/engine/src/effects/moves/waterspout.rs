//! Ports data/moves.ts:20661-20678 (Water Spout). No PRNG draws and no nested events.
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
pub const ID: EffectId = dex::MOVE_WATERSPOUT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_WATERSPOUT_BASEPOWERCALLBACK];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_WATERSPOUT_BASEPOWERCALLBACK => base_power_callback(b, cx),
        _ => panic!("unexpected Water Spout function site"),
    }
}

// data/moves.ts:20665-20669 basePowerCallback(pokemon, target, move): direct call, args
// [pokemon, target, move]. `move.basePower * pokemon.hp / pokemon.maxhp` is returned as the
// unrounded JS number; getDamage floors/clamps it (battle-actions.ts:1585-1589). PRNG: none.
fn base_power_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    support::hp_scaled_base_power(b, cx)
}
