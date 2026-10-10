//! Ports data/moves.ts:5818 (move). No direct PRNG draws; heal events may draw.
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
pub const ID: EffectId = dex::MOVE_FLORALHEALING;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_FLORALHEALING_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:5827-5842. grassy terrain heals modify(maxhp, 0.667) else ceil(maxhp * 0.5); -fail heal + NOT_FAIL. PRNG: none directly.
        dex::HOOK_MOVE_FLORALHEALING_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected floralhealing hook"),
    }
}

fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let grassy = support::terrain_is(b, dex::CONDITION_GRASSYTERRAIN, target);
    let max_hp = b.state.pokemon[target.0 as usize].max_hp as f64;
    let amount = if grassy {
        b.modify(max_hp, 0.667, 1.0)
    } else {
        (max_hp * 0.5).ceil()
    };
    // data/moves.ts:5834-5836 `target.staleness = 'external'` is Endless Battle Clause
    // bookkeeping, which the state model drops (docs/showdown/04-state-model.md:313).
    support::heal_result(b, target, amount)
}
