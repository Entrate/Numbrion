//! Ports data/moves.ts:19497-19518 (Thunderclap). PRNG: none.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_THUNDERCLAP;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_THUNDERCLAP_ONTRY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:19506-19512 onTry(source, target): the same statements as Sucker Punch
        // (cp::sucker_punch_try).
        dex::HOOK_MOVE_THUNDERCLAP_ONTRY => cp::sucker_punch_try(b, cx),
        _ => panic!("unexpected Thunderclap hook"),
    }
}
