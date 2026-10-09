//! Ports data/abilities.ts:5278; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_UNSEENFIST;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_UNSEENFIST_ONMODIFYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_UNSEENFIST_ONMODIFYMOVE => {
            let mov = move_arg(b, cx, 0);
            let mv = b.scratch.moves[mov as usize].as_mut().unwrap();
            if mv.flags & dex::FLAG_CONTACT != 0 {
                mv.flags &= !dex::FLAG_PROTECT;
            }
        }
        _ => unreachable!("unexpected callback for unseenfist"),
    }
    Relay::Undefined
}
