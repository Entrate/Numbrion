//! Ports data/abilities.ts:3089 (ability). No direct PRNG draws; setWeather events may draw.
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
pub const ID: EffectId = dex::ABILITY_ORICHALCUMPULSE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_ORICHALCUMPULSE_ONSTART,
    dex::HOOK_ABILITY_ORICHALCUMPULSE_ONMODIFYATK,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3090-3096. setWeather('sunnyday') truthy -> -activate [source]; else if sunny -> -activate ability. PRNG: none directly.
        dex::HOOK_ABILITY_ORICHALCUMPULSE_ONSTART => on_start(b, cx),
        // data/abilities.ts:3098-3103. sun/Desolate Land (only Sunny Day scoped) -> chainModify([5461, 4096]). PRNG: none.
        dex::HOOK_ABILITY_ORICHALCUMPULSE_ONMODIFYATK => modify_atk(b, cx),
        _ => panic!("unexpected orichalcumpulse hook"),
    }
}

fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    if b.set_weather(dex::CONDITION_SUNNYDAY, Attribution::DEFAULT)
        .truthy()
    {
        b.add(LogEntry::new(
            "-activate",
            &[LogArg::Mon(m), LogArg::Text("Orichalcum Pulse")],
            &[LogTag::Bare("source")],
        ));
    } else if b.is_weather(&[dex::CONDITION_SUNNYDAY]) {
        b.add(LogEntry::new(
            "-activate",
            &[LogArg::Mon(m), LogArg::Text("ability: Orichalcum Pulse")],
            &[],
        ));
    }
    Relay::Undefined
}
fn modify_atk<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 1);
    if support::in_sun(b, m) {
        b.chain_modify(5461.0, 4096.0);
    }
    Relay::Undefined
}
