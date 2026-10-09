//! Ports data/abilities.ts:234 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
#[path = "passivedefense/mod.rs"]
pub(super) mod support;
pub const ID: EffectId = dex::ABILITY_AROMAVEIL;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_AROMAVEIL_ONALLYTRYADDVOLATILE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:235. PRNG: no direct draws.
        dex::HOOK_ABILITY_AROMAVEIL_ONALLYTRYADDVOLATILE => support::try_add_volatile(ID, b, cx),
        _ => panic!("unexpected aromaveil hook"),
    }
}
