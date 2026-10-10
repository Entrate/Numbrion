//! Ports data/moves.ts:12242 (move). No direct PRNG draws; heal events may draw.
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
pub const ID: EffectId = dex::MOVE_MOONLIGHT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_MOONLIGHT_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:12251-12272. Weather factor table, heal(modify(maxhp, factor)). PRNG: none directly.
        dex::HOOK_MOVE_MOONLIGHT_ONHIT => support::sun_heal_move(b, cx),
        _ => panic!("unexpected moonlight hook"),
    }
}
