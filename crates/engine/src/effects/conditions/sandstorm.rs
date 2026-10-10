//! Ports data/conditions.ts:626 (sandstorm weather). Residual Weather events may draw through nested handlers.
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
pub const ID: EffectId = dex::CONDITION_SANDSTORM;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_SANDSTORM_DURATIONCALLBACK,
    dex::HOOK_CONDITION_SANDSTORM_ONMODIFYSPD,
    dex::HOOK_CONDITION_SANDSTORM_ONWEATHER,
    dex::HOOK_CONDITION_SANDSTORM_ONFIELDSTART,
    dex::HOOK_CONDITION_SANDSTORM_ONFIELDRESIDUAL,
    dex::HOOK_CONDITION_SANDSTORM_ONFIELDEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:630-635. `source?.hasItem('smoothrock') ? 8 : 5`; direct call. PRNG: none.
        dex::HOOK_CONDITION_SANDSTORM_DURATIONCALLBACK => {
            support::duration_callback(b, cx, dex::key_ids!("smoothrock"))
        }
        // data/conditions.ts:639-643. Rock type in sandstorm: returns modify(spd, 1.5). PRNG: none directly (Type events).
        dex::HOOK_CONDITION_SANDSTORM_ONMODIFYSPD => {
            support::type_weather_stat(b, cx, support::TYPE_ROCK, dex::CONDITION_SANDSTORM)
        }
        // data/conditions.ts:657-659. damage(baseMaxhp / 16) with event defaults. PRNG: none directly.
        dex::HOOK_CONDITION_SANDSTORM_ONWEATHER => support::sandstorm_weather(b, cx),
        // data/conditions.ts:644-652. -weather Sandstorm (+ [from] ability / [of] source). PRNG: none.
        dex::HOOK_CONDITION_SANDSTORM_ONFIELDSTART => {
            support::weather_field_start(b, cx, "Sandstorm");
            Relay::Undefined
        }
        // data/conditions.ts:653-656. -weather Sandstorm [upkeep], then eachEvent('Weather') when the weather is still active. PRNG: eachEvent speedSort ties and Weather handlers (damage/heal).
        dex::HOOK_CONDITION_SANDSTORM_ONFIELDRESIDUAL => {
            support::weather_residual(b, "Sandstorm", Some(dex::CONDITION_SANDSTORM));
            Relay::Undefined
        }
        // data/conditions.ts:660-662. -weather none. PRNG: none.
        dex::HOOK_CONDITION_SANDSTORM_ONFIELDEND => {
            support::weather_end(b);
            Relay::Undefined
        }
        _ => panic!("unexpected sandstorm hook"),
    }
}
