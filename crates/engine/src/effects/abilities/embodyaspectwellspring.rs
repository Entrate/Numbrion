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
pub const ID: EffectId = dex::ABILITY_EMBODYASPECTWELLSPRING;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_EMBODYASPECTWELLSPRING_ONSTART];
// Payload: 1 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1238-1244. PRNG: boost events only. word0=embodied, bit8=present.
        dex::HOOK_ABILITY_EMBODYASPECTWELLSPRING_ONSTART => {
            let m = mon(b, cx, 0);
            let p = b.state.pokemon[m.0 as usize];
            if p.base_species == dex::SPECIES_OGERPONWELLSPRINGTERA
                && p.terastallized != TypeId::NONE
                && b.hook_state(cx).payload.words[0] == 0
            {
                b.hook_state_mut(cx).payload.words[0] = 1;
                b.hook_state_mut(cx).present |= 1 << 8;
                b.boost(
                    boosts([(Stat::SpD, 1)]),
                    Some(m),
                    Attribution::DEFAULT,
                    false,
                    false,
                );
            }
            Relay::Undefined
        }
        _ => panic!("unexpected embodyaspectwellspring hook"),
    }
}
