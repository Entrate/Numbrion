//! Ports data/moves.ts:3648-3716 (`disable`): onTryHit only (the embedded condition is `conditions/disable.rs`).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_DISABLE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_DISABLE_ONTRYHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:3658-3662 onTryHit(target): `if (!target.lastMove || target.lastMove.isZOrMaxPowered ||
        // target.lastMove.isMax || target.lastMove.id === 'struggle') return false;` Z/Max moves do not exist in
        // this format. Returns false (silent failure after the `|move|` line) or undefined. PRNG: none.
        dex::HOOK_MOVE_DISABLE_ONTRYHIT => {
            let target = mon_arg(b, cx, 0);
            let last_move = b.state.pokemon[target.0 as usize].last_move;
            if last_move == EffectId::NONE || last_move == dex::MOVE_STRUGGLE {
                return Relay::FAIL;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Disable function site"),
    }
}
