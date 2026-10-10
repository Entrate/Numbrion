//! Ports data/conditions.ts:694 (snowscape weather). Residual Weather events may draw through nested handlers.
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
pub const ID: EffectId = dex::CONDITION_SNOWSCAPE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_SNOWSCAPE_DURATIONCALLBACK,
    dex::HOOK_CONDITION_SNOWSCAPE_ONMODIFYDEF,
    dex::HOOK_CONDITION_SNOWSCAPE_ONFIELDSTART,
    dex::HOOK_CONDITION_SNOWSCAPE_ONFIELDRESIDUAL,
    dex::HOOK_CONDITION_SNOWSCAPE_ONFIELDEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:698-703. `source?.hasItem('icyrock') ? 8 : 5`; direct call. PRNG: none.
        dex::HOOK_CONDITION_SNOWSCAPE_DURATIONCALLBACK => {
            support::duration_callback(b, cx, dex::key_ids!("icyrock"))
        }
        // data/conditions.ts:705-709. Ice type in snowscape: returns modify(def, 1.5). PRNG: none directly (Type events).
        dex::HOOK_CONDITION_SNOWSCAPE_ONMODIFYDEF => {
            support::type_weather_stat(b, cx, support::TYPE_ICE, dex::CONDITION_SNOWSCAPE)
        }
        // data/conditions.ts:710-718. -weather Snowscape (+ [from] ability / [of] source). PRNG: none.
        dex::HOOK_CONDITION_SNOWSCAPE_ONFIELDSTART => {
            support::weather_field_start(b, cx, "Snowscape");
            Relay::Undefined
        }
        // data/conditions.ts:719-722. -weather Snowscape [upkeep], then eachEvent('Weather') when the weather is still active. PRNG: eachEvent speedSort ties and Weather handlers (damage/heal).
        dex::HOOK_CONDITION_SNOWSCAPE_ONFIELDRESIDUAL => {
            support::weather_residual(b, "Snowscape", Some(dex::CONDITION_SNOWSCAPE));
            Relay::Undefined
        }
        // data/conditions.ts:723-725. -weather none. PRNG: none.
        dex::HOOK_CONDITION_SNOWSCAPE_ONFIELDEND => {
            support::weather_end(b);
            Relay::Undefined
        }
        _ => panic!("unexpected snowscape hook"),
    }
}
