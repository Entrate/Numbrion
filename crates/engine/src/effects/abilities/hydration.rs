//! Ports data/abilities.ts:1926 (ability). No direct PRNG draws; cureStatus events may draw.
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
pub const ID: EffectId = dex::ABILITY_HYDRATION;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_HYDRATION_ONRESIDUAL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1929-1935. status && rain -> -activate ability: Hydration; cureStatus(). PRNG: none directly.
        dex::HOOK_ABILITY_HYDRATION_ONRESIDUAL => support::hydration_residual(b, cx),
        _ => panic!("unexpected hydration hook"),
    }
}
