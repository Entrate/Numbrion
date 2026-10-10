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
pub const ID: EffectId = dex::ABILITY_SHIELDSDOWN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SHIELDSDOWN_ONSTART,
    dex::HOOK_ABILITY_SHIELDSDOWN_ONRESIDUAL,
    dex::HOOK_ABILITY_SHIELDSDOWN_ONSETSTATUS,
    dex::HOOK_ABILITY_SHIELDSDOWN_ONTRYADDVOLATILE,
];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4242-4253 and 4255-4266. PRNG: forme events only.
        dex::HOOK_ABILITY_SHIELDSDOWN_ONSTART | dex::HOOK_ABILITY_SHIELDSDOWN_ONRESIDUAL => {
            let m = mon(b, cx, 0);
            let p = b.state.pokemon[m.0 as usize];
            if dex::species(p.base_species).base_species != dex::SPECIES_MINIOR
                || p.flags & mon_flags::TRANSFORMED != 0
                || (hook == dex::HOOK_ABILITY_SHIELDSDOWN_ONRESIDUAL && p.hp == 0)
            {
                return Relay::Undefined;
            }
            if p.hp as u32 * 2 > p.max_hp as u32 {
                if p.species != dex::SPECIES_MINIORMETEOR {
                    b.forme_change(
                        m,
                        dex::SPECIES_MINIORMETEOR,
                        b.scratch.current_effect,
                        FormeOptions {
                            permanent: false,
                            message: None,
                        },
                    );
                }
            } else if p.species == dex::SPECIES_MINIORMETEOR {
                let set = b.teams.sides[m.side().0 as usize].sets[(m.0 % 6) as usize].species;
                b.forme_change(
                    m,
                    set,
                    b.scratch.current_effect,
                    FormeOptions {
                        permanent: false,
                        message: None,
                    },
                );
            }
            Relay::Undefined
        }
        // data/abilities.ts:4267-4273. PRNG: none. Always false; only move.status prints immune.
        dex::HOOK_ABILITY_SHIELDSDOWN_ONSETSTATUS => {
            let target = mon(b, cx, 1);
            let p = b.state.pokemon[target.0 as usize];
            if p.species != dex::SPECIES_MINIORMETEOR || p.flags & mon_flags::TRANSFORMED != 0 {
                return Relay::Undefined;
            }
            if effect_has_status(b, effect_at(b, cx, 3)) {
                b.add(LogEntry::new(
                    "-immune",
                    &[LogArg::Mon(target)],
                    &[LogTag::From(EffectRef::Dex(ID))],
                ));
            }
            Relay::Bool(false)
        }
        // data/abilities.ts:4274-4279. PRNG: none. Yawn uses null, not false.
        dex::HOOK_ABILITY_SHIELDSDOWN_ONTRYADDVOLATILE => {
            let target = mon(b, cx, 1);
            let p = b.state.pokemon[target.0 as usize];
            if p.species != dex::SPECIES_MINIORMETEOR
                || p.flags & mon_flags::TRANSFORMED != 0
                || b.event_effect_id(effect_at(b, cx, 0)) != dex::CONDITION_YAWN
            {
                return Relay::Undefined;
            }
            b.add(LogEntry::new(
                "-immune",
                &[LogArg::Mon(target)],
                &[LogTag::From(EffectRef::Dex(ID))],
            ));
            Relay::Null
        }
        _ => panic!("unexpected shieldsdown hook"),
    }
}
