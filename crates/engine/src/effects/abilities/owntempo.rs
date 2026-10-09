//! Ports data/abilities.ts:3145 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_OWNTEMPO;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_OWNTEMPO_ONUPDATE,
    dex::HOOK_ABILITY_OWNTEMPO_ONTRYADDVOLATILE,
    dex::HOOK_ABILITY_OWNTEMPO_ONHIT,
    dex::HOOK_ABILITY_OWNTEMPO_ONTRYBOOST,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3146. PRNG: no direct draws.
        dex::HOOK_ABILITY_OWNTEMPO_ONUPDATE => support::update(ID, b, cx),
        // data/abilities.ts:3152. PRNG: no direct draws.
        dex::HOOK_ABILITY_OWNTEMPO_ONTRYADDVOLATILE => support::try_add_volatile(ID, b, cx),
        // data/abilities.ts:3155. PRNG: no direct draws.
        dex::HOOK_ABILITY_OWNTEMPO_ONHIT => support::hit(ID, b, cx),
        // data/abilities.ts:3160. PRNG: no direct draws.
        dex::HOOK_ABILITY_OWNTEMPO_ONTRYBOOST => support::try_boost(ID, b, cx),
        _ => panic!("unexpected owntempo hook"),
    }
}
