//! Ports data/abilities.ts:1783 (ability). No direct PRNG draws; setTerrain/TryTerrain events may draw.
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
pub const ID: EffectId = dex::ABILITY_HADRONENGINE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_HADRONENGINE_ONSTART,
    dex::HOOK_ABILITY_HADRONENGINE_ONMODIFYSPA,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1784-1788. `!setTerrain('electricterrain') && isTerrain('electricterrain')` -> -activate. PRNG: none directly.
        dex::HOOK_ABILITY_HADRONENGINE_ONSTART => on_start(b, cx),
        // data/abilities.ts:1790-1795. isTerrain('electricterrain') -> chainModify([5461, 4096]). PRNG: none directly.
        dex::HOOK_ABILITY_HADRONENGINE_ONMODIFYSPA => modify_spa(b, cx),
        _ => panic!("unexpected hadronengine hook"),
    }
}

fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    if !b.set_terrain(dex::CONDITION_ELECTRICTERRAIN, Attribution::DEFAULT)
        && support::terrain_is(b, dex::CONDITION_ELECTRICTERRAIN, m)
    {
        b.add(LogEntry::new(
            "-activate",
            &[LogArg::Mon(m), LogArg::Text("ability: Hadron Engine")],
            &[],
        ));
    }
    Relay::Undefined
}
fn modify_spa<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 1);
    if support::terrain_is(b, dex::CONDITION_ELECTRICTERRAIN, m) {
        b.chain_modify(5461.0, 4096.0);
    }
    Relay::Undefined
}
