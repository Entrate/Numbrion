//! Ports data/abilities.ts:5317 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_VITALSPIRIT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_VITALSPIRIT_ONUPDATE,
    dex::HOOK_ABILITY_VITALSPIRIT_ONSETSTATUS,
    dex::HOOK_ABILITY_VITALSPIRIT_ONTRYADDVOLATILE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5318. PRNG: no direct draws.
        dex::HOOK_ABILITY_VITALSPIRIT_ONUPDATE => support::update(ID, b, cx),
        // data/abilities.ts:5324. PRNG: no direct draws.
        dex::HOOK_ABILITY_VITALSPIRIT_ONSETSTATUS => support::set_status(ID, b, cx),
        // data/abilities.ts:5331. PRNG: no direct draws.
        dex::HOOK_ABILITY_VITALSPIRIT_ONTRYADDVOLATILE => support::try_add_volatile(ID, b, cx),
        _ => panic!("unexpected vitalspirit hook"),
    }
}
