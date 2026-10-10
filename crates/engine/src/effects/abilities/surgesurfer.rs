//! Ports data/abilities.ts:4757 (ability). No direct PRNG draws; TryTerrain may run.
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
pub const ID: EffectId = dex::ABILITY_SURGESURFER;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SURGESURFER_ONMODIFYSPE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4758-4762. isTerrain('electricterrain') -> chainModify(2). PRNG: none directly.
        dex::HOOK_ABILITY_SURGESURFER_ONMODIFYSPE => modify_spe(b, cx),
        _ => panic!("unexpected surgesurfer hook"),
    }
}

fn modify_spe<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 1);
    if support::terrain_is(b, dex::CONDITION_ELECTRICTERRAIN, m) {
        b.chain_modify(2.0, 1.0);
    }
    Relay::Undefined
}
