//! Ports data/abilities.ts:4397 (ability). No direct PRNG draws; damage events may draw.
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
pub const ID: EffectId = dex::ABILITY_SOLARPOWER;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SOLARPOWER_ONMODIFYSPA,
    dex::HOOK_ABILITY_SOLARPOWER_ONWEATHER,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4399-4403. sun/Desolate Land (only Sunny Day scoped) -> chainModify(1.5). PRNG: none.
        dex::HOOK_ABILITY_SOLARPOWER_ONMODIFYSPA => modify_spa(b, cx),
        // data/abilities.ts:4404-4409. effectiveWeather === effect.id and sunny -> damage(baseMaxhp / 8, target, target). PRNG: none directly.
        dex::HOOK_ABILITY_SOLARPOWER_ONWEATHER => support::solar_power_weather(b, cx),
        _ => panic!("unexpected solarpower hook"),
    }
}

fn modify_spa<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 1);
    if support::in_sun(b, m) {
        b.chain_modify(1.5, 1.0);
    }
    Relay::Undefined
}
