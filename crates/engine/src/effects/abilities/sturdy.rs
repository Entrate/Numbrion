//! Ports data/abilities.ts:4674 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_STURDY;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_STURDY_ONTRYHIT,
    dex::HOOK_ABILITY_STURDY_ONDAMAGE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4675. PRNG: no direct draws.
        dex::HOOK_ABILITY_STURDY_ONTRYHIT => support::try_hit(ID, b, cx),
        // data/abilities.ts:4682. PRNG: no direct draws.
        dex::HOOK_ABILITY_STURDY_ONDAMAGE => support::damage(ID, b, cx),
        _ => panic!("unexpected sturdy hook"),
    }
}
