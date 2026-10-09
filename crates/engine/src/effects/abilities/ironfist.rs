//! Ports data/abilities.ts:2237; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_IRONFIST;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_IRONFIST_ONBASEPOWER];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_IRONFIST_ONBASEPOWER => {
            let mov = move_arg(b, cx, 3);
            if move_overlay(b, mov).flags & dex::FLAG_PUNCH != 0 {
                b.chain_modify(4915.0, 4096.0);
            }
        }
        _ => unreachable!("unexpected callback for ironfist"),
    }
    Relay::Undefined
}
