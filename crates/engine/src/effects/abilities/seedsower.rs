//! Ports data/abilities.ts:4120 (ability). No direct PRNG draws; setTerrain events may draw.
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
pub const ID: EffectId = dex::ABILITY_SEEDSOWER;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SEEDSOWER_ONDAMAGINGHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4121-4123. setTerrain('grassyterrain'); the event target is the holder. PRNG: none directly.
        dex::HOOK_ABILITY_SEEDSOWER_ONDAMAGINGHIT => {
            support::set_terrain_default(b, dex::CONDITION_GRASSYTERRAIN)
        }
        _ => panic!("unexpected seedsower hook"),
    }
}
