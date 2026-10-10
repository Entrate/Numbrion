//! Ports data/abilities.ts:91 (airlock). No direct PRNG draws; eachEvent WeatherChange may draw via speedSort ties.
#![allow(unused_imports)]
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
#[path = "weatherterrain/mod.rs"]
pub(super) mod support;
pub const ID: EffectId = dex::ABILITY_AIRLOCK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_AIRLOCK_ONSWITCHIN,
    dex::HOOK_ABILITY_AIRLOCK_ONSTART,
    dex::HOOK_ABILITY_AIRLOCK_ONEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:91-95. Announces, then calls onStart directly (same effect). PRNG: as onStart.
        dex::HOOK_ABILITY_AIRLOCK_ONSWITCHIN => {
            support::suppress_weather_switch_in(b, cx, "Air Lock")
        }
        // data/abilities.ts:96-99. abilityState.ending = false; eachEvent(WeatherChange). PRNG: speedSort ties in eachEvent.
        dex::HOOK_ABILITY_AIRLOCK_ONSTART => support::suppress_weather_start(b, mon_arg(b, cx, 0)),
        // data/abilities.ts:100-103. abilityState.ending = true; eachEvent(WeatherChange). PRNG: speedSort ties in eachEvent.
        dex::HOOK_ABILITY_AIRLOCK_ONEND => support::suppress_weather_end(b, mon_arg(b, cx, 0)),
        _ => panic!("unexpected airlock hook"),
    }
}
