#![allow(unused_imports, unused_variables)]
//! Pinned data/rulesets.ts `speciesclause` onBegin. PRNG: none.
use crate::{Battle, dex::{self, HookId}, effects::HookWaiver, event::{HookCtx, Relay}, ids::EffectId, log::{LogSink, LogArg, LogEntry}};
pub const ID: EffectId = dex::RULE_SPECIESCLAUSE;
pub const HOOKS: &[HookId] = &[dex::HOOK_RULE_SPECIESCLAUSE_ONBEGIN];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[HookWaiver {hook:dex::HOOK_RULE_SPECIESCLAUSE_ONVALIDATETEAM,reason:crate::effects::WaiverReason::ValidationOnly,explanation:"rulesets.ts:795-805 onValidateTeam is called by TeamValidator, never Battle; packed teams are supplied by the pinned oracle"}];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, _: HookCtx) -> Relay {
    assert_eq!(hook,dex::HOOK_RULE_SPECIESCLAUSE_ONBEGIN);
    b.add(LogEntry::new("rule", &[LogArg::Text("Species Clause: Limit one of each Pokémon")], &[]));
    Relay::Undefined
}
