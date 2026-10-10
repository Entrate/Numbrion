//! Ports data/conditions.ts:544 (sunnyday weather). Residual Weather events may draw through nested handlers.
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
pub const ID: EffectId = dex::CONDITION_SUNNYDAY;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_SUNNYDAY_DURATIONCALLBACK,
    dex::HOOK_CONDITION_SUNNYDAY_ONWEATHERMODIFYDAMAGE,
    dex::HOOK_CONDITION_SUNNYDAY_ONIMMUNITY,
    dex::HOOK_CONDITION_SUNNYDAY_ONFIELDSTART,
    dex::HOOK_CONDITION_SUNNYDAY_ONFIELDRESIDUAL,
    dex::HOOK_CONDITION_SUNNYDAY_ONFIELDEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:548-553. `source?.hasItem('heatrock') ? 8 : 5`; direct call. PRNG: none.
        dex::HOOK_CONDITION_SUNNYDAY_DURATIONCALLBACK => {
            support::duration_callback(b, cx, "heatrock")
        }
        // data/conditions.ts:554-568. Hydro Steam attacker branch is outside scope; defender sun: Fire x1.5, Water x0.5. PRNG: none.
        dex::HOOK_CONDITION_SUNNYDAY_ONWEATHERMODIFYDAMAGE => support::weather_modify_damage(
            b,
            cx,
            dex::CONDITION_SUNNYDAY,
            support::TYPE_FIRE,
            support::TYPE_WATER,
        ),
        // data/conditions.ts:577-581. effectiveWeather === sunnyday and type === 'frz' -> false. PRNG: none.
        dex::HOOK_CONDITION_SUNNYDAY_ONIMMUNITY => immunity(b, cx),
        // data/conditions.ts:569-576. -weather SunnyDay (+ [from] ability / [of] source). PRNG: none.
        dex::HOOK_CONDITION_SUNNYDAY_ONFIELDSTART => {
            support::weather_field_start(b, cx, "SunnyDay");
            Relay::Undefined
        }
        // data/conditions.ts:582-585. -weather SunnyDay [upkeep], then eachEvent('Weather'). PRNG: eachEvent speedSort ties and Weather handlers (damage/heal).
        dex::HOOK_CONDITION_SUNNYDAY_ONFIELDRESIDUAL => {
            support::weather_residual(b, "SunnyDay", None);
            Relay::Undefined
        }
        // data/conditions.ts:586-588. -weather none. PRNG: none.
        dex::HOOK_CONDITION_SUNNYDAY_ONFIELDEND => {
            support::weather_end(b);
            Relay::Undefined
        }
        _ => panic!("unexpected sunnyday hook"),
    }
}

fn immunity<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 1);
    if b.effective_weather(m) != dex::CONDITION_SUNNYDAY {
        return Relay::Undefined;
    }
    if support::immunity_is(b, cx, ImmunityId::Frz) {
        return Relay::Bool(false);
    }
    Relay::Undefined
}
