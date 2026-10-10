//! Ports data/abilities.ts:2292 (ability). No direct PRNG draws.
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
pub const ID: EffectId = dex::ABILITY_LEAFGUARD;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_LEAFGUARD_ONSETSTATUS,
    dex::HOOK_ABILITY_LEAFGUARD_ONTRYADDVOLATILE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:2293-2300. sun -> (effect.status ? -immune) and return false. PRNG: none.
        dex::HOOK_ABILITY_LEAFGUARD_ONSETSTATUS => set_status(b, cx),
        // data/abilities.ts:2301-2306. yawn && sun -> -immune and return null. PRNG: none.
        dex::HOOK_ABILITY_LEAFGUARD_ONTRYADDVOLATILE => try_add_volatile(b, cx),
        _ => panic!("unexpected leafguard hook"),
    }
}

fn immune<L: LogSink>(b: &mut Battle<L>, target: MonId) {
    b.add(LogEntry::new(
        "-immune",
        &[LogArg::Mon(target)],
        &[LogTag::From(EffectRef::Dex(ID))],
    ));
}
fn set_status<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 1);
    if support::in_sun(b, target) {
        let effect = support::effect_ref(b, cx, 3);
        if support::has_status_field(b, effect) {
            immune(b, target);
        }
        return Relay::Bool(false);
    }
    Relay::Undefined
}
fn try_add_volatile<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 1);
    if support::effect_id(b, cx, 0) == dex::CONDITION_YAWN && support::in_sun(b, target) {
        immune(b, target);
        return Relay::Null;
    }
    Relay::Undefined
}
