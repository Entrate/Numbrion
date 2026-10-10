//! Ports data/abilities.ts:1078 (ability). No direct PRNG draws; setWeather events may draw.
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
pub const ID: EffectId = dex::ABILITY_DRIZZLE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_DRIZZLE_ONSTART];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1079-1082. Kyogre holding Blue Orb returns; else setWeather('raindance'). PRNG: none directly.
        dex::HOOK_ABILITY_DRIZZLE_ONSTART => support::set_weather_on_start(
            b,
            cx,
            dex::CONDITION_RAINDANCE,
            Some((dex::SPECIES_KYOGRE, "blueorb")),
        ),
        _ => panic!("unexpected drizzle hook"),
    }
}
