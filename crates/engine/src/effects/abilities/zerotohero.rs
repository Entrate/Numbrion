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
pub const ID: EffectId = dex::ABILITY_ZEROTOHERO;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_ZEROTOHERO_ONSWITCHOUT,
    dex::HOOK_ABILITY_ZEROTOHERO_ONSWITCHIN,
];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5629-5635. PRNG: forme events only; heroMessageDisplayed is a Pokemon flag.
        dex::HOOK_ABILITY_ZEROTOHERO_ONSWITCHOUT => {
            let m = mon(b, cx, 0);
            let p = b.state.pokemon[m.0 as usize];
            if dex::species(p.base_species).base_species != dex::SPECIES_PALAFIN {
                return Relay::Undefined;
            }
            if p.species != dex::SPECIES_PALAFINHERO {
                b.forme_change(
                    m,
                    dex::SPECIES_PALAFINHERO,
                    EffectRef::Dex(ID),
                    FormeOptions {
                        permanent: true,
                        message: None,
                    },
                );
                b.state.pokemon[m.0 as usize].flags &= !mon_flags::HERO_MESSAGE;
            }
            Relay::Undefined
        }
        // data/abilities.ts:5636-5642. PRNG: none. Flag survives ability-state replacement.
        dex::HOOK_ABILITY_ZEROTOHERO_ONSWITCHIN => {
            let m = mon(b, cx, 0);
            let p = b.state.pokemon[m.0 as usize];
            if dex::species(p.base_species).base_species != dex::SPECIES_PALAFIN {
                return Relay::Undefined;
            }
            if p.flags & mon_flags::HERO_MESSAGE == 0 && p.species == dex::SPECIES_PALAFINHERO {
                b.add(LogEntry::new(
                    "-activate",
                    &[LogArg::Mon(m), LogArg::EffectFullName(EffectRef::Dex(ID))],
                    &[],
                ));
                b.state.pokemon[m.0 as usize].flags |= mon_flags::HERO_MESSAGE;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected zerotohero hook"),
    }
}
