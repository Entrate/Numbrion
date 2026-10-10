//! Ports data/moves.ts:18078-18091 (Stone Axe onAfterHit + onAfterSubDamage).
//! Payload: none. No direct PRNG draws; the Stealth Rock installation events retain their own draws.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_STONEAXE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_STONEAXE_ONAFTERHIT,
    dex::HOOK_MOVE_STONEAXE_ONAFTERSUBDAMAGE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:18078 onAfterHit(target, source, move): unless move.hasSheerForce, every foe
        // side gets addSideCondition('stealthrock') with no explicit source. Returns undefined.
        dex::HOOK_MOVE_STONEAXE_ONAFTERHIT => {
            support::add_foe_hazard(b, cx, dex::CONDITION_STEALTHROCK, false)
        }
        // data/moves.ts:18085 onAfterSubDamage(damage, target, source, move): same body behind a
        // Substitute, with an extra `source.hp` check. Returns undefined.
        dex::HOOK_MOVE_STONEAXE_ONAFTERSUBDAMAGE => {
            support::add_foe_hazard(b, cx, dex::CONDITION_STEALTHROCK, true)
        }
        _ => panic!("unexpected Stone Axe function site"),
    }
}
