//! Ports data/moves.ts:7655 (move). No direct PRNG draws.
#![allow(unused_imports)]
use crate::effects::registry::abilities_airlock::support;
use crate::{
    Battle,
    actions::{Attribution, HealEffect, MoveHandle},
    dex::{self, HookId, ImmunityId, MoveTarget},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::scratch::MoveAccuracy,
};
pub const ID: EffectId = dex::MOVE_GRASSYGLIDE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_GRASSYGLIDE_ONMODIFYPRIORITY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:7664-7668. grassy terrain && source.isGrounded() -> priority + 1. PRNG: none directly.
        dex::HOOK_MOVE_GRASSYGLIDE_ONMODIFYPRIORITY => modify_priority(b, cx),
        _ => panic!("unexpected grassyglide hook"),
    }
}

fn modify_priority<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let priority = support::number_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    if support::terrain_is(b, dex::CONDITION_GRASSYTERRAIN, source) && support::grounded(b, source)
    {
        return Relay::Number(priority + 1.0);
    }
    Relay::Undefined
}
