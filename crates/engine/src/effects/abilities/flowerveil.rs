//! Ports data/abilities.ts:1420 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_FLOWERVEIL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_FLOWERVEIL_ONALLYTRYBOOST,
    dex::HOOK_ABILITY_FLOWERVEIL_ONALLYSETSTATUS,
    dex::HOOK_ABILITY_FLOWERVEIL_ONALLYTRYADDVOLATILE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1421. PRNG: no direct draws.
        dex::HOOK_ABILITY_FLOWERVEIL_ONALLYTRYBOOST => support::try_boost(ID, b, cx),
        // data/abilities.ts:1436. PRNG: no direct draws.
        dex::HOOK_ABILITY_FLOWERVEIL_ONALLYSETSTATUS => support::set_status(ID, b, cx),
        // data/abilities.ts:1446. PRNG: no direct draws.
        dex::HOOK_ABILITY_FLOWERVEIL_ONALLYTRYADDVOLATILE => support::try_add_volatile(ID, b, cx),
        _ => panic!("unexpected flowerveil hook"),
    }
}
