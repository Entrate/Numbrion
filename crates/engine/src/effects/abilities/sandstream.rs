//! Ports data/abilities.ts:3998 (ability). No direct PRNG draws; setWeather events may draw.
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
pub const ID: EffectId = dex::ABILITY_SANDSTREAM;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SANDSTREAM_ONSTART];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3999-4001. setWeather('sandstorm'). PRNG: none directly.
        dex::HOOK_ABILITY_SANDSTREAM_ONSTART => {
            support::set_weather_on_start(b, cx, dex::CONDITION_SANDSTORM, None)
        }
        _ => panic!("unexpected sandstream hook"),
    }
}
