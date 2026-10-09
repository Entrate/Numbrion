//! Ports data/abilities.ts:43; no direct PRNG draws. Core queries retain their event semantics.
use crate::{Battle,dex::{self,HookId},effects::{HookWaiver,support::*},event::{HookCtx,Relay},ids::*,log::LogSink};
pub const ID:EffectId=dex::ABILITY_ADAPTABILITY;
pub const HOOKS:&[HookId]=&[dex::HOOK_ABILITY_ADAPTABILITY_ONMODIFYSTAB];
pub const PAYLOAD_WORDS:usize=0;
pub const WAIVERS:&[HookWaiver]=&[];
pub fn dispatch<L:LogSink>(hook:HookId,b:&mut Battle<L>,cx:HookCtx)->Relay {
match hook {
dex::HOOK_ABILITY_ADAPTABILITY_ONMODIFYSTAB => {let stab = relay_number(b,cx,0); let m=mon_arg(b,cx,1); let mov=move_arg(b,cx,3); let mv=*move_overlay(b,mov);
 if mv.traits & dex::MOVE_TRAIT_FORCESTAB != 0 || b.has_type(m,&[mv.move_type]) {return Relay::Number(if stab==2.0 {2.25}else{2.0});}},
 _ => unreachable!("unexpected callback for adaptability"),
}
Relay::Undefined
}

#[cfg(test)]
#[path="passiveoffense/tests.rs"]
mod tests;
