//! Condition view of data/rulesets.ts:1379-1403, shares the exact Rule callbacks.
use crate::{Battle,dex::{self,HookId},effects::HookWaiver,event::{HookCtx,Relay},ids::EffectId,log::LogSink};
pub const ID:EffectId=dex::CONDITION_SLEEPCLAUSEMOD;
pub const HOOKS:&[HookId]=&[dex::HOOK_CONDITION_SLEEPCLAUSEMOD_ONBEGIN,dex::HOOK_CONDITION_SLEEPCLAUSEMOD_ONSETSTATUS];
pub const PAYLOAD_WORDS:usize=0;
pub const WAIVERS:&[HookWaiver]=&[];
pub fn dispatch<L:LogSink>(hook:HookId,b:&mut Battle<L>,cx:HookCtx)->Relay {
    match hook {
        dex::HOOK_CONDITION_SLEEPCLAUSEMOD_ONBEGIN => crate::effects::registry::rules_sleepclausemod::begin(b),
        dex::HOOK_CONDITION_SLEEPCLAUSEMOD_ONSETSTATUS => crate::effects::registry::rules_sleepclausemod::set_status(b,cx),
        _=>panic!("unexpected Sleep Clause condition hook"),
    }
}
