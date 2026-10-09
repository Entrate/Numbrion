//! Ports data/abilities.ts:1859; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_HEAVYMETAL;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_HEAVYMETAL_ONMODIFYWEIGHT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_HEAVYMETAL_ONMODIFYWEIGHT => {
            Relay::Number(relay_number(b, cx, 0) * 2.0)
        }
        _ => unreachable!("unexpected callback for heavymetal"),
    }
}
