//! Ports data/abilities.ts:5204 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_TURBOBLAZE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_TURBOBLAZE_ONSTART,
    dex::HOOK_ABILITY_TURBOBLAZE_ONMODIFYMOVE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5205. PRNG: no direct draws.
        dex::HOOK_ABILITY_TURBOBLAZE_ONSTART => support::start(ID, b, cx),
        // data/abilities.ts:5208. PRNG: no direct draws.
        dex::HOOK_ABILITY_TURBOBLAZE_ONMODIFYMOVE => support::modify_move(ID, b, cx),
        _ => panic!("unexpected turboblaze hook"),
    }
}
