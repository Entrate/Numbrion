//! Ports data/abilities.ts:1887; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_HUGEPOWER;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_HUGEPOWER_ONMODIFYATK];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_HUGEPOWER_ONMODIFYATK => {
            b.chain_modify(2.0, 1.0);
        }
        _ => unreachable!("unexpected callback for hugepower"),
    }
    Relay::Undefined
}
