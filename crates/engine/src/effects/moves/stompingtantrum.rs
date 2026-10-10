//! Ports data/moves.ts:18049-18068 (Stomping Tantrum). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
    state::ResultFlag,
};
#[path = "movecallbacks/mod.rs"]
mod support;
pub const ID: EffectId = dex::MOVE_STOMPINGTANTRUM;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_STOMPINGTANTRUM_BASEPOWERCALLBACK];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_STOMPINGTANTRUM_BASEPOWERCALLBACK => base_power_callback(b, cx),
        _ => panic!("unexpected Stomping Tantrum function site"),
    }
}

// data/moves.ts:18053-18059 basePowerCallback(pokemon, target, move): direct call, args
// [pokemon, target, move]. `pokemon.moveLastTurnResult === false` is strict: only an explicit
// false doubles the live `move.basePower` (undefined and null results do not). PRNG: none.
fn base_power_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let base_power = support::live_base_power(b, cx, 2);
    if b.state.pokemon[pokemon.0 as usize].move_last_turn_result == ResultFlag::False {
        return Relay::Number(base_power * 2.0);
    }
    Relay::Number(base_power)
}
