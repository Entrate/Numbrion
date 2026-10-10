//! Ports data/moves.ts:10048-10066 (Lash Out). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
    state::mon_flags,
};
pub const ID: EffectId = dex::MOVE_LASHOUT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_LASHOUT_ONBASEPOWER];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_LASHOUT_ONBASEPOWER => on_base_power(b, cx),
        _ => panic!("unexpected Lash Out function site"),
    }
}

// data/moves.ts:10057-10062 onBasePower(basePower, source): runEvent('BasePower', attacker,
// defender, move, basePower, true) passes [basePower, attacker, defender, move]
// (battle-actions.ts:1637). `source.statsLoweredThisTurn` then `this.chainModify(2)`, which
// mutates the frame and returns undefined. PRNG: none.
fn on_base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let source = mon_arg(b, cx, 1);
    if b.state.pokemon[source.0 as usize].flags & mon_flags::STATS_LOWERED != 0 {
        b.chain_modify(2.0, 1.0);
    }
    Relay::Undefined
}
