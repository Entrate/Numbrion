//! Ports data/moves.ts:20852 (move). No direct PRNG draws.
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
pub const ID: EffectId = dex::MOVE_WILDBOLTSTORM;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_WILDBOLTSTORM_ONMODIFYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:20861-20865. `target && rain -> move.accuracy = true`. PRNG: none.
        dex::HOOK_MOVE_WILDBOLTSTORM_ONMODIFYMOVE => support::rain_always_hits(b, cx),
        _ => panic!("unexpected wildboltstorm hook"),
    }
}
