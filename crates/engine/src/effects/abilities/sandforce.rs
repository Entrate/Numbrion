//! Ports data/abilities.ts:3957 (ability). No direct PRNG draws.
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
pub const ID: EffectId = dex::ABILITY_SANDFORCE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SANDFORCE_ONBASEPOWER,
    dex::HOOK_ABILITY_SANDFORCE_ONIMMUNITY,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3959-3966. isWeather('sandstorm') && Rock/Ground/Steel move -> chainModify([5325, 4096]). PRNG: none.
        dex::HOOK_ABILITY_SANDFORCE_ONBASEPOWER => base_power(b, cx),
        // data/abilities.ts:3967-3969. type === 'sandstorm' -> false. PRNG: none.
        dex::HOOK_ABILITY_SANDFORCE_ONIMMUNITY => immunity(b, cx),
        _ => panic!("unexpected sandforce hook"),
    }
}

fn base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if b.is_weather(&[dex::CONDITION_SANDSTORM]) {
        let ty = support::move_type(b, cx, 3);
        if ty == support::TYPE_ROCK || ty == support::TYPE_GROUND || ty == support::TYPE_STEEL {
            b.chain_modify(5325.0, 4096.0);
        }
    }
    Relay::Undefined
}
fn immunity<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if support::immunity_is(b, cx, ImmunityId::Sandstorm) {
        return Relay::Bool(false);
    }
    Relay::Undefined
}
