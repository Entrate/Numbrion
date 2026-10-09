//! Ports data/abilities.ts:3438 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_PRESSURE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_PRESSURE_ONSTART,
    dex::HOOK_ABILITY_PRESSURE_ONDEDUCTPP,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3439. PRNG: no direct draws.
        dex::HOOK_ABILITY_PRESSURE_ONSTART => support::start(ID, b, cx),
        // data/abilities.ts:3442. PRNG: no direct draws.
        dex::HOOK_ABILITY_PRESSURE_ONDEDUCTPP => support::deduct_pp(ID, b, cx),
        _ => panic!("unexpected pressure hook"),
    }
}
