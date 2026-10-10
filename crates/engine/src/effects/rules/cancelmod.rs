#![allow(unused_imports, unused_variables)]
//! Pinned data/rulesets.ts `cancelmod` onBegin. PRNG: none.
use crate::{Battle, dex::{self, HookId}, effects::HookWaiver, event::{HookCtx, Relay}, ids::EffectId, log::{LogSink, LogArg, LogEntry}};
pub const ID: EffectId = dex::RULE_CANCELMOD;
pub const HOOKS: &[HookId] = &[dex::HOOK_RULE_CANCELMOD_ONBEGIN];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, _: HookCtx) -> Relay {
    assert_eq!(hook,dex::HOOK_RULE_CANCELMOD_ONBEGIN);
    b.state.support_cancel = true;
    Relay::Undefined
}
