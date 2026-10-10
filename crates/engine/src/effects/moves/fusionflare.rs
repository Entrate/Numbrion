//! Ports data/moves.ts:6372-6390 (Fusion Flare). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_FUSIONFLARE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_FUSIONFLARE_ONBASEPOWER];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_FUSIONFLARE_ONBASEPOWER => on_base_power(b, cx),
        _ => panic!("unexpected Fusion Flare function site"),
    }
}

// data/moves.ts:6381-6386 onBasePower(basePower, pokemon): runEvent('BasePower', attacker,
// defender, move, basePower, true) passes [basePower, attacker, defender, move]
// (battle-actions.ts:1637). `this.lastSuccessfulMoveThisTurn === 'fusionbolt'` (battle.ts:169,
// set at battle-actions.ts:309 and cleared at battle.ts:1629 -> BattleState.last_successful_move)
// then `this.chainModify(2)`, which mutates the frame and returns undefined. PRNG: none.
fn on_base_power<L: LogSink>(b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    if b.state.last_successful_move == dex::MOVE_FUSIONBOLT {
        b.chain_modify(2.0, 1.0);
    }
    Relay::Undefined
}
