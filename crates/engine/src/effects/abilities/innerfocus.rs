//! Ports data/abilities.ts:2154 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_INNERFOCUS;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_INNERFOCUS_ONTRYADDVOLATILE,
    dex::HOOK_ABILITY_INNERFOCUS_ONTRYBOOST,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:2155. PRNG: no direct draws.
        dex::HOOK_ABILITY_INNERFOCUS_ONTRYADDVOLATILE => support::try_add_volatile(ID, b, cx),
        // data/abilities.ts:2158. PRNG: no direct draws.
        dex::HOOK_ABILITY_INNERFOCUS_ONTRYBOOST => support::try_boost(ID, b, cx),
        _ => panic!("unexpected innerfocus hook"),
    }
}
