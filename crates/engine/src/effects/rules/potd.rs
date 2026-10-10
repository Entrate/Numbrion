#![allow(unused_imports, unused_variables)]
//! Pinned data/rulesets.ts `potd` onBegin. PRNG: none.
use crate::{Battle, dex::{self, HookId}, effects::HookWaiver, event::{HookCtx, Relay}, ids::EffectId, log::{LogSink, LogArg, LogEntry}};
pub const ID: EffectId = dex::RULE_POTD;
pub const HOOKS: &[HookId] = &[dex::HOOK_RULE_POTD_ONBEGIN];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, _: HookCtx) -> Relay {
    assert_eq!(hook,dex::HOOK_RULE_POTD_ONBEGIN);
    // rulesets.ts:488-490: Config?.potd is absent in the pinned in-process oracle.
    // Teams are pre-generated; Numbrion has no server-global Pokemon-of-the-Day setting.
    Relay::Undefined
}
