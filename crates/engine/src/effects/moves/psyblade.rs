//! Ports data/moves.ts:14023 (move). No direct PRNG draws.
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
pub const ID: EffectId = dex::MOVE_PSYBLADE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_PSYBLADE_ONBASEPOWER];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:14032-14037. electric terrain -> chainModify(1.5) (no groundedness test). PRNG: none directly.
        dex::HOOK_MOVE_PSYBLADE_ONBASEPOWER => base_power(b, cx),
        _ => panic!("unexpected psyblade hook"),
    }
}

fn base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let source = mon_arg(b, cx, 1);
    if support::terrain_is(b, dex::CONDITION_ELECTRICTERRAIN, source) {
        b.chain_modify(1.5, 1.0);
    }
    Relay::Undefined
}
