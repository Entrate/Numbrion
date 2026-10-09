//! Ports data/abilities.ts:4918; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_TECHNICIAN;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_TECHNICIAN_ONBASEPOWER];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_TECHNICIAN_ONBASEPOWER => {
            let power = relay_number(b, cx, 0);
            let modifier = b.event_frame(cx).expect("BasePower frame").modifier;
            if b.modify(power, modifier as f64, 4096.0) <= 60.0 {
                b.chain_modify(1.5, 1.0);
            }
        }
        _ => unreachable!("unexpected callback for technician"),
    }
    Relay::Undefined
}
