//! Ports data/abilities.ts:4203; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_SHEERFORCE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SHEERFORCE_ONMODIFYMOVE,
    dex::HOOK_ABILITY_SHEERFORCE_ONBASEPOWER,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_SHEERFORCE_ONMODIFYMOVE => {
            let mov = move_arg(b, cx, 0);
            let mv = b.scratch.moves[mov as usize].as_mut().unwrap();
            if mv.secondaries_present && mv.traits & dex::MOVE_TRAIT_HASSHEERFORCEBOOST == 0 {
                mv.secondaries_present = false;
                mv.secondary_count = 0;
                mv.secondaries = [None; 4];
                mv.self_effect = None; /* Clangorous Soulblaze is outside this format. */
                mv.runtime_flags |= crate::state::scratch::move_runtime::HAS_SHEER_FORCE;
            }
        }
        dex::HOOK_ABILITY_SHEERFORCE_ONBASEPOWER => {
            let mov = move_arg(b, cx, 3);
            let mv = move_overlay(b, mov);
            if mv.runtime_flags & crate::state::scratch::move_runtime::HAS_SHEER_FORCE != 0
                || mv.traits & dex::MOVE_TRAIT_HASSHEERFORCEBOOST != 0
            {
                b.chain_modify(5325.0, 4096.0);
            }
        }
        _ => unreachable!("unexpected callback for sheerforce"),
    }
    Relay::Undefined
}
