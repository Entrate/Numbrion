//! Ports data/moves.ts:20692 (move). No direct PRNG draws.
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
pub const ID: EffectId = dex::MOVE_WEATHERBALL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_WEATHERBALL_ONMODIFYTYPE,
    dex::HOOK_MOVE_WEATHERBALL_ONMODIFYMOVE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:20701-20719. effectiveWeather -> Fire/Water/Rock/Ice (Desolate Land, Primordial Sea, Hail outside scope). PRNG: none.
        dex::HOOK_MOVE_WEATHERBALL_ONMODIFYTYPE => modify_type(b, cx),
        // data/moves.ts:20720-20739. In sun, rain, sandstorm and snow: basePower *= 2. PRNG: none.
        dex::HOOK_MOVE_WEATHERBALL_ONMODIFYMOVE => modify_move(b, cx),
        _ => panic!("unexpected weatherball hook"),
    }
}

fn modify_type<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_arg(b, cx, 0);
    let user = mon_arg(b, cx, 1);
    let weather = b.effective_weather(user);
    let ty = if weather == dex::CONDITION_SUNNYDAY {
        support::TYPE_FIRE
    } else if weather == dex::CONDITION_RAINDANCE {
        support::TYPE_WATER
    } else if weather == dex::CONDITION_SANDSTORM {
        support::TYPE_ROCK
    } else if weather == dex::CONDITION_SNOWSCAPE {
        support::TYPE_ICE
    } else {
        return Relay::Undefined;
    };
    b.active_move_mut(MoveHandle(mv)).move_type = ty;
    Relay::Undefined
}
fn modify_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_arg(b, cx, 0);
    let user = mon_arg(b, cx, 1);
    let weather = b.effective_weather(user);
    if weather == dex::CONDITION_SUNNYDAY
        || weather == dex::CONDITION_RAINDANCE
        || weather == dex::CONDITION_SANDSTORM
        || weather == dex::CONDITION_SNOWSCAPE
    {
        b.active_move_mut(MoveHandle(mv)).base_power *= 2.0;
    }
    // `this.debug(...)` produces no protocol output.
    Relay::Undefined
}
