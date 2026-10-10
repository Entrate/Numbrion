//! Ports data/abilities.ts:4348 (ability). No direct PRNG draws.
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
pub const ID: EffectId = dex::ABILITY_SLUSHRUSH;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SLUSHRUSH_ONMODIFYSPE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4349-4353. isWeather(['hail','snowscape']) -> chainModify(2); Hail is outside scope. PRNG: none.
        dex::HOOK_ABILITY_SLUSHRUSH_ONMODIFYSPE => modify_spe(b),
        _ => panic!("unexpected slushrush hook"),
    }
}

fn modify_spe<L: LogSink>(b: &mut Battle<L>) -> Relay {
    if b.is_weather(&[dex::CONDITION_SNOWSCAPE]) {
        b.chain_modify(2.0, 1.0);
    }
    Relay::Undefined
}
