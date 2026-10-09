//! Ports data/abilities.ts:1771; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_GUTS;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_GUTS_ONMODIFYATK];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_GUTS_ONMODIFYATK => {
            let m = mon_arg(b, cx, 1);
            if b.state.pokemon[m.0 as usize].status != crate::state::Status::None {
                b.chain_modify(1.5, 1.0);
            }
        }
        _ => unreachable!("unexpected callback for guts"),
    }
    Relay::Undefined
}
