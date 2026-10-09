//! Ports data/abilities.ts:4768; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_SWARM;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SWARM_ONMODIFYATK,
    dex::HOOK_ABILITY_SWARM_ONMODIFYSPA,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_SWARM_ONMODIFYATK | dex::HOOK_ABILITY_SWARM_ONMODIFYSPA => {
            let source = mon_arg(b, cx, 1);
            let mov = move_arg(b, cx, 3);
            let p = b.state.pokemon[source.0 as usize];
            if move_overlay(b, mov).move_type == TypeId(7) && (p.hp as f64) <= p.max_hp as f64 / 3.0
            {
                b.chain_modify(1.5, 1.0);
            }
        }
        _ => unreachable!("unexpected callback for swarm"),
    }
    Relay::Undefined
}
