//! Ports data/abilities.ts:4477; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_STAKEOUT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_STAKEOUT_ONMODIFYATK,
    dex::HOOK_ABILITY_STAKEOUT_ONMODIFYSPA,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_STAKEOUT_ONMODIFYATK | dex::HOOK_ABILITY_STAKEOUT_ONMODIFYSPA => {
            let target = mon_arg(b, cx, 2);
            if b.state.pokemon[target.0 as usize].active_turns == 0 {
                b.chain_modify(2.0, 1.0);
            }
        }
        _ => unreachable!("unexpected callback for stakeout"),
    }
    Relay::Undefined
}
