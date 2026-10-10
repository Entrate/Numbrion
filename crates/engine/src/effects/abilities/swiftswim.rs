//! Ports data/abilities.ts:4810 (ability). No direct PRNG draws.
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
pub const ID: EffectId = dex::ABILITY_SWIFTSWIM;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SWIFTSWIM_ONMODIFYSPE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4811-4815. rain/Primordial Sea (only Rain Dance is scoped) -> chainModify(2). PRNG: none.
        dex::HOOK_ABILITY_SWIFTSWIM_ONMODIFYSPE => modify_spe(b, cx),
        _ => panic!("unexpected swiftswim hook"),
    }
}

fn modify_spe<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 1);
    if support::in_rain(b, m) {
        b.chain_modify(2.0, 1.0);
    }
    Relay::Undefined
}
