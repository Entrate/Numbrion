//! Ports data/moves.ts:2229-2242 (Ceaseless Edge onAfterHit + onAfterSubDamage).
//! Payload: none. No direct PRNG draws; the Spikes installation events retain their own draws.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_CEASELESSEDGE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_CEASELESSEDGE_ONAFTERHIT,
    dex::HOOK_MOVE_CEASELESSEDGE_ONAFTERSUBDAMAGE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:2229 onAfterHit(target, source, move): singleEvent('AfterHit', moveData, {}, t,
        // pokemon, move) after the damaging hit (battle-actions.ts:1116). Unless Sheer Force removed
        // the effect (move.hasSheerForce), every foe side gets addSideCondition('spikes') with no
        // explicit source. Returns undefined.
        dex::HOOK_MOVE_CEASELESSEDGE_ONAFTERHIT => {
            support::add_foe_hazard(b, cx, dex::CONDITION_SPIKES, false)
        }
        // data/moves.ts:2236 onAfterSubDamage(damage, target, source, move): same body behind a
        // Substitute, with an extra `source.hp` check (data/moves.ts:18368 fires it). Returns undefined.
        dex::HOOK_MOVE_CEASELESSEDGE_ONAFTERSUBDAMAGE => {
            support::add_foe_hazard(b, cx, dex::CONDITION_SPIKES, true)
        }
        _ => panic!("unexpected Ceaseless Edge function site"),
    }
}
