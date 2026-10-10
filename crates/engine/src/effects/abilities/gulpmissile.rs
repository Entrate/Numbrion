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
pub const ID: EffectId = dex::ABILITY_GULPMISSILE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_GULPMISSILE_ONDAMAGINGHIT,
    dex::HOOK_ABILITY_GULPMISSILE_ONSOURCETRYPRIMARYHIT,
];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1747-1759. PRNG: damage, boost/status and forme events only.
        dex::HOOK_ABILITY_GULPMISSILE_ONDAMAGINGHIT => {
            let target = mon(b, cx, 1);
            let source = mon(b, cx, 2);
            let s = b.state.pokemon[source.0 as usize];
            if s.hp == 0 || s.flags & mon_flags::ACTIVE == 0 || b.is_semi_invulnerable(target) {
                return Relay::Undefined;
            }
            let species = b.state.pokemon[target.0 as usize].species;
            if matches!(
                species,
                dex::SPECIES_CRAMORANTGULPING | dex::SPECIES_CRAMORANTGORGING
            ) {
                b.damage(
                    s.max_hp as f64 / 4.,
                    Some(source),
                    Attribution::from_move(target, EffectRef::None),
                );
                if b.state.pokemon[target.0 as usize].species == dex::SPECIES_CRAMORANTGULPING {
                    b.boost(
                        boosts([(Stat::Def, -1)]),
                        Some(source),
                        Attribution::from_move(target, EffectRef::None),
                        true,
                        false,
                    );
                } else {
                    b.try_set_status(
                        source,
                        dex::CONDITION_PAR,
                        Attribution::from_move(target, effect_at(b, cx, 3)),
                    );
                }
                b.forme_change(
                    target,
                    dex::SPECIES_CRAMORANT,
                    effect_at(b, cx, 3),
                    FormeOptions {
                        permanent: false,
                        message: None,
                    },
                );
            }
            Relay::Undefined
        }
        // data/abilities.ts:1761-1766. PRNG: forme events only; damage hit may never occur.
        dex::HOOK_ABILITY_GULPMISSILE_ONSOURCETRYPRIMARYHIT => {
            let source = mon(b, cx, 1);
            let effect = effect_at(b, cx, 2);
            if b.event_effect_id(effect) == dex::MOVE_SURF
                && b.has_ability(source, &[ID])
                && b.state.pokemon[source.0 as usize].species == dex::SPECIES_CRAMORANT
            {
                let p = b.state.pokemon[source.0 as usize];
                let species = if p.hp as u32 * 2 <= p.max_hp as u32 {
                    dex::SPECIES_CRAMORANTGORGING
                } else {
                    dex::SPECIES_CRAMORANTGULPING
                };
                b.forme_change(
                    source,
                    species,
                    effect,
                    FormeOptions {
                        permanent: false,
                        message: None,
                    },
                );
            }
            Relay::Undefined
        }
        _ => panic!("unexpected gulpmissile hook"),
    }
}
