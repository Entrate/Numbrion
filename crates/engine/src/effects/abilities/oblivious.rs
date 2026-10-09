//! Ports data/abilities.ts:3009 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_OBLIVIOUS;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_OBLIVIOUS_ONUPDATE,
    dex::HOOK_ABILITY_OBLIVIOUS_ONIMMUNITY,
    dex::HOOK_ABILITY_OBLIVIOUS_ONTRYHIT,
    dex::HOOK_ABILITY_OBLIVIOUS_ONTRYBOOST,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3010. PRNG: no direct draws.
        dex::HOOK_ABILITY_OBLIVIOUS_ONUPDATE => support::update(ID, b, cx),
        // data/abilities.ts:3022. PRNG: no direct draws.
        dex::HOOK_ABILITY_OBLIVIOUS_ONIMMUNITY => support::immunity(ID, b, cx),
        // data/abilities.ts:3025. PRNG: no direct draws.
        dex::HOOK_ABILITY_OBLIVIOUS_ONTRYHIT => support::try_hit(ID, b, cx),
        // data/abilities.ts:3031. PRNG: no direct draws.
        dex::HOOK_ABILITY_OBLIVIOUS_ONTRYBOOST => support::try_boost(ID, b, cx),
        _ => panic!("unexpected oblivious hook"),
    }
}
