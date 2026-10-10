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
pub const ID: EffectId = dex::ABILITY_TERASHIFT;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_TERASHIFT_ONSWITCHIN];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4968-4974. PRNG: forme/ability events only. Fires on SwitchIn, not Start.
        dex::HOOK_ABILITY_TERASHIFT_ONSWITCHIN => {
            let m = mon(b, cx, 0);
            let p = b.state.pokemon[m.0 as usize];
            if dex::species(p.base_species).base_species != dex::SPECIES_TERAPAGOS {
                return Relay::Undefined;
            }
            if p.species != dex::SPECIES_TERAPAGOSTERASTAL {
                b.add(LogEntry::new(
                    "-activate",
                    &[LogArg::Mon(m), LogArg::EffectFullName(EffectRef::Dex(ID))],
                    &[],
                ));
                b.forme_change(
                    m,
                    dex::SPECIES_TERAPAGOSTERASTAL,
                    EffectRef::Dex(ID),
                    FormeOptions {
                        permanent: true,
                        message: None,
                    },
                );
            }
            Relay::Undefined
        }
        _ => panic!("unexpected terashift hook"),
    }
}
