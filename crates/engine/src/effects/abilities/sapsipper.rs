//! Ports data/abilities.ts:4024 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_SAPSIPPER;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SAPSIPPER_ONTRYHIT,
    dex::HOOK_ABILITY_SAPSIPPER_ONALLYTRYHITSIDE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4026. PRNG: no direct draws.
        dex::HOOK_ABILITY_SAPSIPPER_ONTRYHIT => support::try_hit(ID, b, cx),
        // data/abilities.ts:4034. PRNG: no direct draws.
        dex::HOOK_ABILITY_SAPSIPPER_ONALLYTRYHITSIDE => support::try_hit_side(ID, b, cx),
        _ => panic!("unexpected sapsipper hook"),
    }
}
