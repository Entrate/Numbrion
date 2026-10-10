//! Ports data/moves.ts:6049-6051 (move Follow Me onTry). Its volatile condition is
//! conditions/followme.rs.
//! Payload: none. Direct PRNG draws: none.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_FOLLOWME;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_FOLLOWME_ONTRY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, _b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:6049-6051 onTry(source): `return this.activePerHalf > 1`, a real boolean.
        // gen9randomdoublesbattle is a doubles game (activePerHalf 2), so it is always true.
        // PRNG: none.
        dex::HOOK_MOVE_FOLLOWME_ONTRY => Relay::Bool(support::ACTIVE_PER_HALF > 1),
        _ => panic!("unexpected Follow Me function site"),
    }
}
