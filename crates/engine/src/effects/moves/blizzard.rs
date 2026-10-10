//! Ports data/moves.ts:1491 (move). No direct PRNG draws.
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
pub const ID: EffectId = dex::MOVE_BLIZZARD;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_BLIZZARD_ONMODIFYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:1500-1502. `isWeather([hail, snowscape]) -> move.accuracy = true`; Hail is outside the scoped dex. PRNG: none.
        dex::HOOK_MOVE_BLIZZARD_ONMODIFYMOVE => modify_move(b, cx),
        _ => panic!("unexpected blizzard hook"),
    }
}

fn modify_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_arg(b, cx, 0);
    if b.is_weather(&[dex::CONDITION_SNOWSCAPE]) {
        b.active_move_mut(MoveHandle(mv)).accuracy = MoveAccuracy::Always;
    }
    Relay::Undefined
}
