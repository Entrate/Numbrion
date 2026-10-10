//! Ports data/moves.ts:18396-18417 (Sucker Punch). PRNG: none.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_SUCKERPUNCH;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_SUCKERPUNCH_ONTRY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:18405-18412 onTry(source, target): fails (false) unless the target still
        // has a queued, non-Status (Me First excepted) move and is not recharging. The body is
        // shared with Thunderclap (cp::sucker_punch_try documents the exact source statements).
        dex::HOOK_MOVE_SUCKERPUNCH_ONTRY => cp::sucker_punch_try(b, cx),
        _ => panic!("unexpected Sucker Punch hook"),
    }
}
