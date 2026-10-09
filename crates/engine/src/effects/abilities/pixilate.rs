//! Ports data/abilities.ts:3297 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_PIXILATE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_PIXILATE_ONMODIFYTYPE,
    dex::HOOK_ABILITY_PIXILATE_ONBASEPOWER,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3299. PRNG: no direct draws.
        dex::HOOK_ABILITY_PIXILATE_ONMODIFYTYPE => support::modify_type(ID, b, cx),
        // data/abilities.ts:3310. PRNG: no direct draws.
        dex::HOOK_ABILITY_PIXILATE_ONBASEPOWER => support::base_power(ID, b, cx),
        _ => panic!("unexpected pixilate hook"),
    }
}
