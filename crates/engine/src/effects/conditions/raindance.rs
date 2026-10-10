//! Ports data/conditions.ts:474 (raindance weather). Residual Weather events may draw through nested handlers.
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
pub const ID: EffectId = dex::CONDITION_RAINDANCE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_RAINDANCE_DURATIONCALLBACK,
    dex::HOOK_CONDITION_RAINDANCE_ONWEATHERMODIFYDAMAGE,
    dex::HOOK_CONDITION_RAINDANCE_ONFIELDSTART,
    dex::HOOK_CONDITION_RAINDANCE_ONFIELDRESIDUAL,
    dex::HOOK_CONDITION_RAINDANCE_ONFIELDEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:478-483. `source?.hasItem('damprock') ? 8 : 5`; direct call. PRNG: none.
        dex::HOOK_CONDITION_RAINDANCE_DURATIONCALLBACK => {
            support::duration_callback(b, cx, dex::key_ids!("damprock"))
        }
        // data/conditions.ts:484-494. defender.effectiveWeather() === rain: Water x1.5, Fire x0.5 via chainModify. PRNG: none.
        dex::HOOK_CONDITION_RAINDANCE_ONWEATHERMODIFYDAMAGE => support::weather_modify_damage(
            b,
            cx,
            dex::CONDITION_RAINDANCE,
            support::TYPE_WATER,
            support::TYPE_FIRE,
        ),
        // data/conditions.ts:495-503. -weather RainDance (+ [from] ability / [of] source). PRNG: none.
        dex::HOOK_CONDITION_RAINDANCE_ONFIELDSTART => {
            support::weather_field_start(b, cx, "RainDance");
            Relay::Undefined
        }
        // data/conditions.ts:504-507. -weather RainDance [upkeep], then eachEvent('Weather'). PRNG: eachEvent speedSort ties and Weather handlers (damage/heal).
        dex::HOOK_CONDITION_RAINDANCE_ONFIELDRESIDUAL => {
            support::weather_residual(b, "RainDance", None);
            Relay::Undefined
        }
        // data/conditions.ts:508-510. -weather none. PRNG: none.
        dex::HOOK_CONDITION_RAINDANCE_ONFIELDEND => {
            support::weather_end(b);
            Relay::Undefined
        }
        _ => panic!("unexpected raindance hook"),
    }
}
