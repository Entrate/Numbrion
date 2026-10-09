//! Ports data/abilities.ts:5154; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_TRANSISTOR;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_TRANSISTOR_ONMODIFYATK,
    dex::HOOK_ABILITY_TRANSISTOR_ONMODIFYSPA,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_TRANSISTOR_ONMODIFYATK | dex::HOOK_ABILITY_TRANSISTOR_ONMODIFYSPA => {
            let mov = move_arg(b, cx, 3);
            if move_overlay(b, mov).move_type == TypeId(13) {
                b.chain_modify(5325.0, 4096.0);
            }
        }
        _ => unreachable!("unexpected callback for transistor"),
    }
    Relay::Undefined
}
