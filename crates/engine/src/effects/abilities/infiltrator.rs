//! Ports data/abilities.ts:2132; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_INFILTRATOR;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_INFILTRATOR_ONMODIFYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_INFILTRATOR_ONMODIFYMOVE => {
            let mov = move_arg(b, cx, 0);
            b.scratch.moves[mov as usize]
                .as_mut()
                .unwrap()
                .runtime_flags |= crate::state::scratch::move_runtime::INFILTRATES;
        }
        _ => unreachable!("unexpected callback for infiltrator"),
    }
    Relay::Undefined
}
