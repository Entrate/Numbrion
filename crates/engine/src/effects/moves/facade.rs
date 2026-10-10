//! Ports data/moves.ts:5032-5049 (Facade). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
    state::Status,
};
pub const ID: EffectId = dex::MOVE_FACADE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_FACADE_ONBASEPOWER];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_FACADE_ONBASEPOWER => on_base_power(b, cx),
        _ => panic!("unexpected Facade function site"),
    }
}

// data/moves.ts:5041-5045 onBasePower(basePower, pokemon): runEvent('BasePower', attacker,
// defender, move, basePower, true) passes [basePower, attacker, defender, move]
// (battle-actions.ts:1637). `pokemon.status && pokemon.status !== 'slp'` then
// `this.chainModify(2)`, which mutates the frame and returns undefined. PRNG: none.
fn on_base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 1);
    let status = b.state.pokemon[pokemon.0 as usize].status;
    if status != Status::None && status != Status::Sleep {
        b.chain_modify(2.0, 1.0);
    }
    Relay::Undefined
}
