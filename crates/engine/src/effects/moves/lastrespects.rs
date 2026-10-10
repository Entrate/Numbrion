//! Ports data/moves.ts:10091-10106 (Last Respects). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_LASTRESPECTS;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_LASTRESPECTS_BASEPOWERCALLBACK];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_LASTRESPECTS_BASEPOWERCALLBACK => base_power_callback(b, cx),
        _ => panic!("unexpected Last Respects function site"),
    }
}

// data/moves.ts:10095-10097 basePowerCallback(pokemon, target, move): direct call, args
// [pokemon, target, move]. `50 + 50 * pokemon.side.totalFainted` (side.ts totalFainted counts
// this side's fainted Pokemon). PRNG: none.
fn base_power_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let fainted = b.state.sides[pokemon.side().0 as usize].total_fainted;
    Relay::Number(50.0 + 50.0 * f64::from(fainted))
}
