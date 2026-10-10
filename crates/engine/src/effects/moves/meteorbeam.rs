//! Ports data/moves.ts:11739-11763 (Meteor Beam). PRNG: no direct draws; the boost events,
//! ChargeMove (Power Herb) and the twoturnmove Start run nested core events.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_METEORBEAM;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_METEORBEAM_ONTRYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:11748-11759 onTryMove(attacker, defender, move): same body as Electro
        // Shot without the rain short-cut: removeVolatile(move.id) -> return; -prepare;
        // boost({spa: 1}, attacker, attacker, move); runEvent('ChargeMove') falsy -> return;
        // addVolatile('twoturnmove', defender); return null.
        dex::HOOK_MOVE_METEORBEAM_ONTRYMOVE => cp::charge_try_move(b, cx, true, None),
        _ => panic!("unexpected Meteor Beam hook"),
    }
}
