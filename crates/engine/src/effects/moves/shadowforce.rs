//! Ports data/moves.ts:16073-16107 (Shadow Force; its `condition` has only the constants
//! `duration: 2` / `onInvulnerability: false`, so the embedded condition has no file). PRNG: no
//! direct draws; ChargeMove (Power Herb) and the twoturnmove Start run nested core events.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_SHADOWFORCE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_SHADOWFORCE_ONTRYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:16083-16093 onTryMove(attacker, defender, move): removeVolatile(move.id)
        // -> return; -prepare; runEvent('ChargeMove') falsy -> return; addVolatile('twoturnmove',
        // defender); return null.
        dex::HOOK_MOVE_SHADOWFORCE_ONTRYMOVE => cp::charge_try_move(b, cx, false, None),
        _ => panic!("unexpected Shadow Force hook"),
    }
}
