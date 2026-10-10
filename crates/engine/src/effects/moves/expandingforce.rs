//! Ports data/moves.ts:4943 (move). No direct PRNG draws; Type/TryTerrain events may run.
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
pub const ID: EffectId = dex::MOVE_EXPANDINGFORCE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_EXPANDINGFORCE_ONBASEPOWER,
    dex::HOOK_MOVE_EXPANDINGFORCE_ONMODIFYMOVE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:4952-4957. psychic terrain && source.isGrounded() -> chainModify(1.5). PRNG: none directly.
        dex::HOOK_MOVE_EXPANDINGFORCE_ONBASEPOWER => base_power(b, cx),
        // data/moves.ts:4958-4962. psychic terrain && source.isGrounded() -> move.target = 'allAdjacentFoes'. PRNG: none directly.
        dex::HOOK_MOVE_EXPANDINGFORCE_ONMODIFYMOVE => modify_move(b, cx),
        _ => panic!("unexpected expandingforce hook"),
    }
}

fn base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let source = mon_arg(b, cx, 1);
    if support::terrain_is(b, dex::CONDITION_PSYCHICTERRAIN, source) && support::grounded(b, source)
    {
        b.chain_modify(1.5, 1.0);
    }
    Relay::Undefined
}
fn modify_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    if support::terrain_is(b, dex::CONDITION_PSYCHICTERRAIN, source) && support::grounded(b, source)
    {
        b.active_move_mut(MoveHandle(mv)).target = MoveTarget::AllAdjacentFoes;
    }
    Relay::Undefined
}
