//! TeamValidator-only function: rulesets.ts:246-481, unreachable from Battle.
use crate::{Battle,dex::{self,HookId},effects::{HookWaiver,WaiverReason},event::{HookCtx,Relay},ids::EffectId,log::LogSink};
pub const ID:EffectId=dex::RULE_OBTAINABLEMISC;
pub const HOOKS:&[HookId]=&[];
pub const PAYLOAD_WORDS:usize=0;
pub const WAIVERS:&[HookWaiver]=&[HookWaiver {hook:dex::HOOK_RULE_OBTAINABLEMISC_ONCHANGESET,reason:WaiverReason::ValidationOnly,explanation:"rulesets.ts:246-481: TeamValidator callback, never called by battle simulator; pinned oracle supplies packed teams"}];
pub fn dispatch<L:LogSink>(_:HookId,_:&mut Battle<L>,_:HookCtx)->Relay{panic!("validation-only callback reached")}
