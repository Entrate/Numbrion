//! Ports data/moves.ts:16361 (move). No direct PRNG draws; heal events may draw.
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
pub const ID: EffectId = dex::MOVE_SHOREUP;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_SHOREUP_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:16370-16381. sandstorm 0.667 else 0.5, heal(modify(maxhp, factor)). PRNG: none directly.
        dex::HOOK_MOVE_SHOREUP_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected shoreup hook"),
    }
}

fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    let factor = if b.is_weather(&[dex::CONDITION_SANDSTORM]) {
        0.667
    } else {
        0.5
    };
    support::heal_fraction_move(b, m, factor)
}
