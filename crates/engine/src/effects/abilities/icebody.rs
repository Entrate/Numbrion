//! Ports data/abilities.ts:1956 (ability). No direct PRNG draws; heal events may draw.
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
pub const ID: EffectId = dex::ABILITY_ICEBODY;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_ICEBODY_ONWEATHER,
    dex::HOOK_ABILITY_ICEBODY_ONIMMUNITY,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1957-1961. effect.id hail/snowscape -> heal(baseMaxhp / 16); Hail is outside scope. PRNG: none directly.
        dex::HOOK_ABILITY_ICEBODY_ONWEATHER => support::ice_body_weather(b, cx),
        // data/abilities.ts:1962-1964. type === 'hail' -> false. PRNG: none.
        dex::HOOK_ABILITY_ICEBODY_ONIMMUNITY => immunity(b, cx),
        _ => panic!("unexpected icebody hook"),
    }
}

fn immunity<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if support::immunity_is(b, cx, ImmunityId::Hail) {
        return Relay::Bool(false);
    }
    Relay::Undefined
}
