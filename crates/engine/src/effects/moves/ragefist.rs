//! Ports data/moves.ts:14583-14598 (Rage Fist). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_RAGEFIST;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_RAGEFIST_BASEPOWERCALLBACK];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_RAGEFIST_BASEPOWERCALLBACK => base_power_callback(b, cx),
        _ => panic!("unexpected Rage Fist function site"),
    }
}

// data/moves.ts:14587-14589 basePowerCallback(pokemon): direct call, args
// [pokemon, target, move]. `Math.min(350, 50 + 50 * pokemon.timesAttacked)`. PRNG: none.
fn base_power_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let times_attacked = f64::from(b.state.pokemon[pokemon.0 as usize].times_attacked);
    Relay::Number((50.0 + 50.0 * times_attacked).min(350.0))
}
