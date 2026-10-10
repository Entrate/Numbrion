//! Ports data/moves.ts:4630-4659 (Electro Shot). PRNG: no direct draws; the boost events,
//! ChargeMove (Power Herb) and the twoturnmove Start run nested core events.
#[path = "chargepriority/mod.rs"]
pub(super) mod cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_ELECTROSHOT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_ELECTROSHOT_ONTRYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:4639-4655 onTryMove(attacker, defender, move): removeVolatile(move.id) ->
        // return; -prepare; boost({spa: 1}, attacker, attacker, move); rain/primordialsea ->
        // [still] + -anim, return; runEvent('ChargeMove') falsy -> return; addVolatile('twoturnmove',
        // defender); return null. (Primordial Sea is outside the generated scope.)
        dex::HOOK_MOVE_ELECTROSHOT_ONTRYMOVE => {
            cp::charge_try_move(b, cx, true, Some(dex::CONDITION_RAINDANCE))
        }
        _ => panic!("unexpected Electro Shot hook"),
    }
}
