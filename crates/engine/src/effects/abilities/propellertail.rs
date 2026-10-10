//! Ports data/abilities.ts:3487-3497 (Propeller Tail onModifyMove). No PRNG draws.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_PROPELLERTAIL;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_PROPELLERTAIL_ONMODIFYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3489-3492 onModifyMove(move), onModifyMovePriority 1:
        // `move.tracksTarget = move.target !== 'scripted'` (always true here). Most of the effect
        // lives in Battle#getTarget (targeting.rs also keys it off this ability, because ModifyMove
        // runs after getTarget). Returns undefined. PRNG: none.
        dex::HOOK_ABILITY_PROPELLERTAIL_ONMODIFYMOVE => support::modify_move_tracks_target(b, cx),
        _ => panic!("unexpected Propeller Tail function site"),
    }
}
