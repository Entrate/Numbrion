//! Ports data/moves.ts:4610-4629 (Electro Drift). No direct PRNG draws; the Effectiveness
//! events dispatched by `runEffectiveness` may sort ties.
use crate::{
    Battle,
    actions::MoveHandle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_ELECTRODRIFT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_ELECTRODRIFT_ONBASEPOWER];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_ELECTRODRIFT_ONBASEPOWER => on_base_power(b, cx),
        _ => panic!("unexpected Electro Drift function site"),
    }
}

// data/moves.ts:4619-4625 onBasePower(basePower, source, target, move): runEvent('BasePower',
// attacker, defender, move, basePower, true) passes [basePower, attacker, defender, move]
// (battle-actions.ts:1637). `target.runEffectiveness(move) > 0` then
// `this.chainModify([5461, 4096])`, which mutates the frame and returns undefined.
// PRNG: none directly.
fn on_base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 2);
    let handle = MoveHandle(move_arg(b, cx, 3));
    if b.run_effectiveness(target, handle) > 0 {
        b.chain_modify(5461.0, 4096.0);
    }
    Relay::Undefined
}
