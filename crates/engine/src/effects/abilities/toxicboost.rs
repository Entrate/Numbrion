//! Ports data/abilities.ts:5080; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_TOXICBOOST;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_TOXICBOOST_ONBASEPOWER];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_TOXICBOOST_ONBASEPOWER => {
            let source = mon_arg(b, cx, 1);
            let mov = move_arg(b, cx, 3);
            if matches!(
                b.state.pokemon[source.0 as usize].status,
                crate::state::Status::Poison | crate::state::Status::Toxic
            ) && move_overlay(b, mov).category == dex::Category::Physical
            {
                b.chain_modify(1.5, 1.0);
            }
        }
        _ => unreachable!("unexpected callback for toxicboost"),
    }
    Relay::Undefined
}
