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
pub const ID: EffectId = dex::MOVE_RELICSONG;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_RELICSONG_ONAFTERMOVESECONDARYSELF];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:14949-14954. PRNG: forme events only; secondary sleep draw belongs to move core.
        dex::HOOK_MOVE_RELICSONG_ONAFTERMOVESECONDARYSELF => {
            let m = mon(b, cx, 0);
            let p = b.state.pokemon[m.0 as usize];
            if dex::species(p.base_species).base_species == dex::SPECIES_MELOETTA
                && p.flags & mon_flags::TRANSFORMED == 0
            {
                let species = if p.species == dex::SPECIES_MELOETTAPIROUETTE {
                    dex::SPECIES_MELOETTA
                } else {
                    dex::SPECIES_MELOETTAPIROUETTE
                };
                b.forme_change(
                    m,
                    species,
                    EffectRef::Dex(ID),
                    FormeOptions {
                        permanent: false,
                        message: Some("[msg]"),
                    },
                );
            }
            Relay::Undefined
        }
        _ => panic!("unexpected relicsong hook"),
    }
}
