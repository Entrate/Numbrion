//! Ports data/abilities.ts:4504-4514 (Stalwart onModifyMove). No PRNG draws.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_STALWART;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_STALWART_ONMODIFYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4506-4509 onModifyMove(move), onModifyMovePriority 1:
        // `move.tracksTarget = move.target !== 'scripted'` (always true here). Most of the effect
        // lives in Battle#getTarget (targeting.rs also keys it off this ability, because ModifyMove
        // runs after getTarget). Returns undefined. PRNG: none.
        dex::HOOK_ABILITY_STALWART_ONMODIFYMOVE => support::modify_move_tracks_target(b, cx),
        _ => panic!("unexpected Stalwart function site"),
    }
}
