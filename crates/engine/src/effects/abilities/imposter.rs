//! Pinned Showdown 7332b60 effect port. See handler comments for source and draws.
#![allow(unused_imports)]
use crate::effects::registry::abilities_battlebond::support::{
    boosts, effect_at, effect_has_status, effect_is_move, foe, mon_opt, type_named,
};
use crate::{
    Battle,
    actions::{
        Attribution, FormeOptions, ImmunityMessage, ImmunitySource, MoveHandle, Stat, StatOptions,
        Types,
    },
    dex::{self, Category, DataValue, HookId, MoveTarget},
    effects::{
        HookWaiver,
        support::{mon_arg as mon, move_arg, optional_id, relay_number},
    },
    event::{CallArgs, EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::{
        mon_flags,
        scratch::{MoveEffectsScratch, move_runtime},
    },
};
pub const ID: EffectId = dex::ABILITY_IMPOSTER;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_IMPOSTER_ONSWITCHIN];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:2117-2126. PRNG: nested Transform events only.
        dex::HOOK_ABILITY_IMPOSTER_ONSWITCHIN => {
            let user = mon(b, cx, 0);
            let p = b.state.pokemon[user.0 as usize];
            let target = b.state.sides[1 - user.side().0 as usize].active[1 - p.position as usize];
            if target != MonId::NONE {
                b.transform_into(user, target, EffectRef::Dex(ID));
            }
            Relay::Undefined
        }
        _ => panic!("unexpected imposter hook"),
    }
}
