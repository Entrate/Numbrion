//! Ports data/abilities.ts:2169 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_INSOMNIA;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_INSOMNIA_ONUPDATE,
    dex::HOOK_ABILITY_INSOMNIA_ONSETSTATUS,
    dex::HOOK_ABILITY_INSOMNIA_ONTRYADDVOLATILE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:2170. PRNG: no direct draws.
        dex::HOOK_ABILITY_INSOMNIA_ONUPDATE => support::update(ID, b, cx),
        // data/abilities.ts:2176. PRNG: no direct draws.
        dex::HOOK_ABILITY_INSOMNIA_ONSETSTATUS => support::set_status(ID, b, cx),
        // data/abilities.ts:2183. PRNG: no direct draws.
        dex::HOOK_ABILITY_INSOMNIA_ONTRYADDVOLATILE => support::try_add_volatile(ID, b, cx),
        _ => panic!("unexpected insomnia hook"),
    }
}
