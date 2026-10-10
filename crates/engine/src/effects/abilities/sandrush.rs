//! Ports data/abilities.ts:3975 (ability). No direct PRNG draws.
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
pub const ID: EffectId = dex::ABILITY_SANDRUSH;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SANDRUSH_ONMODIFYSPE,
    dex::HOOK_ABILITY_SANDRUSH_ONIMMUNITY,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3976-3980. isWeather('sandstorm') -> chainModify(2). PRNG: none.
        dex::HOOK_ABILITY_SANDRUSH_ONMODIFYSPE => modify_spe(b),
        // data/abilities.ts:3981-3983. type === 'sandstorm' -> false. PRNG: none.
        dex::HOOK_ABILITY_SANDRUSH_ONIMMUNITY => immunity(b, cx),
        _ => panic!("unexpected sandrush hook"),
    }
}

fn modify_spe<L: LogSink>(b: &mut Battle<L>) -> Relay {
    if b.is_weather(&[dex::CONDITION_SANDSTORM]) {
        b.chain_modify(2.0, 1.0);
    }
    Relay::Undefined
}
fn immunity<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if support::immunity_is(b, cx, ImmunityId::Sandstorm) {
        return Relay::Bool(false);
    }
    Relay::Undefined
}
