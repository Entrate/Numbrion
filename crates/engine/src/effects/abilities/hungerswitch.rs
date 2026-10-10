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
pub const ID: EffectId = dex::ABILITY_HUNGERSWITCH;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_HUNGERSWITCH_ONRESIDUAL];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1899-1903. PRNG: forme events only. Default source is this.effect.
        dex::HOOK_ABILITY_HUNGERSWITCH_ONRESIDUAL => {
            let m = mon(b, cx, 0);
            let p = b.state.pokemon[m.0 as usize];
            if dex::species(p.species).base_species != dex::SPECIES_MORPEKO
                || p.terastallized != TypeId::NONE
            {
                return Relay::Undefined;
            }
            let species = if p.species == dex::SPECIES_MORPEKO {
                dex::SPECIES_MORPEKOHANGRY
            } else {
                dex::SPECIES_MORPEKO
            };
            b.forme_change(
                m,
                species,
                b.scratch.current_effect,
                FormeOptions {
                    permanent: false,
                    message: None,
                },
            );
            Relay::Undefined
        }
        _ => panic!("unexpected hungerswitch hook"),
    }
}
