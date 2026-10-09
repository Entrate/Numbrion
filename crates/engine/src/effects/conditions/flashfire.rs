//! Ports data/abilities.ts:1342 (embedded condition). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_FLASHFIRE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_FLASHFIRE_ONSTART,
    dex::HOOK_CONDITION_FLASHFIRE_ONMODIFYATK,
    dex::HOOK_CONDITION_FLASHFIRE_ONMODIFYSPA,
    dex::HOOK_CONDITION_FLASHFIRE_ONEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1357. PRNG: no direct draws.
        dex::HOOK_CONDITION_FLASHFIRE_ONSTART => support::start(ID, b, cx),
        // data/abilities.ts:1361. PRNG: no direct draws.
        dex::HOOK_CONDITION_FLASHFIRE_ONMODIFYATK => support::modify_atk(ID, b, cx),
        // data/abilities.ts:1368. PRNG: no direct draws.
        dex::HOOK_CONDITION_FLASHFIRE_ONMODIFYSPA => support::modify_sp_a(ID, b, cx),
        // data/abilities.ts:1374. PRNG: no direct draws.
        dex::HOOK_CONDITION_FLASHFIRE_ONEND => support::end(ID, b, cx),
        _ => panic!("unexpected flashfire hook"),
    }
}
