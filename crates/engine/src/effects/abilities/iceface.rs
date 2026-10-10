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
pub const ID: EffectId = dex::ABILITY_ICEFACE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_ICEFACE_ONSTART,
    dex::HOOK_ABILITY_ICEFACE_ONDAMAGE,
    dex::HOOK_ABILITY_ICEFACE_ONCRITICALHIT,
    dex::HOOK_ABILITY_ICEFACE_ONEFFECTIVENESS,
    dex::HOOK_ABILITY_ICEFACE_ONUPDATE,
    dex::HOOK_ABILITY_ICEFACE_ONWEATHERCHANGE,
];
// Payload: 1 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1973-1979. PRNG: forme events only.
        dex::HOOK_ABILITY_ICEFACE_ONSTART => restore(b, cx),
        // data/abilities.ts:2011-2020. PRNG: environment/forme events only.
        dex::HOOK_ABILITY_ICEFACE_ONWEATHERCHANGE => {
            let effect = effect_at(b, cx, 2);
            if b.event_effect_type(effect) == dex::EffectType::Ability
                && dex::ABILITIES[(b.event_effect_id(effect).0 - dex::ABILITY_START) as usize]
                    .suppress_weather
            {
                return Relay::Undefined;
            }
            let m = mon(b, cx, 0);
            if b.state.pokemon[m.0 as usize].hp == 0 {
                return Relay::Undefined;
            }
            restore(b, cx)
        } // data/abilities.ts:1981-1987. PRNG: none; numeric zero absorbs a hit.
        // word0=busted, bit8=present (true on damage, false on Ice Face restoration).
        dex::HOOK_ABILITY_ICEFACE_ONDAMAGE => {
            let target = mon(b, cx, 1);
            let effect = effect_at(b, cx, 3);
            if effect_is_move(b, effect)
                && physical(b, effect)
                && b.state.pokemon[target.0 as usize].species == dex::SPECIES_EISCUE
            {
                b.add(LogEntry::new(
                    "-activate",
                    &[
                        LogArg::Mon(target),
                        LogArg::EffectFullName(EffectRef::Dex(ID)),
                    ],
                    &[],
                ));
                b.hook_state_mut(cx).payload.words[0] = 1;
                b.hook_state_mut(cx).present |= 1 << 8;
                return Relay::Number(0.);
            }
            Relay::Undefined
        }
        // data/abilities.ts:1988-1995. PRNG: runImmunity events only.
        dex::HOOK_ABILITY_ICEFACE_ONCRITICALHIT => shield(b, cx, 2, false),
        // data/abilities.ts:1996-2005. PRNG: runImmunity events only.
        dex::HOOK_ABILITY_ICEFACE_ONEFFECTIVENESS => shield(b, cx, 3, true),
        // data/abilities.ts:2006-2010. PRNG: forme events only.
        dex::HOOK_ABILITY_ICEFACE_ONUPDATE => {
            let m = mon(b, cx, 0);
            if b.state.pokemon[m.0 as usize].species == dex::SPECIES_EISCUE
                && b.hook_state(cx).payload.words[0] != 0
            {
                b.forme_change(
                    m,
                    dex::SPECIES_EISCUENOICE,
                    EffectRef::Dex(ID),
                    FormeOptions {
                        permanent: true,
                        message: None,
                    },
                );
            }
            Relay::Undefined
        }
        _ => panic!("unexpected iceface hook"),
    }
}

// data/abilities.ts:1990, 1997. PRNG: runImmunity events only.
fn shield<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, index: usize, effectiveness: bool) -> Relay {
    let Some(target) = mon_opt(b, cx, if effectiveness { 1 } else { 0 }) else {
        return Relay::Undefined;
    };
    let h = MoveHandle(move_arg(b, cx, index));
    let mv = *b.active_move(h);
    if b.state.pokemon[target.0 as usize].species != dex::SPECIES_EISCUE
        || mv.category != Category::Physical
    {
        return Relay::Undefined;
    }
    if b.get_volatile(target, dex::CONDITION_SUBSTITUTE).is_some()
        && mv.flags & dex::FLAG_BYPASSSUB == 0
        && mv.runtime_flags & move_runtime::INFILTRATES == 0
    {
        return Relay::Undefined;
    }
    if !b.run_immunity(target, ImmunitySource::Move(h), ImmunityMessage::Silent) {
        return Relay::Undefined;
    }
    if effectiveness {
        Relay::Number(0.)
    } else {
        Relay::Bool(false)
    }
}

// data/abilities.ts:1973-1979,2015-2020. PRNG: weather/forme events only.
fn restore<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon(b, cx, 0);
    if b.is_weather(&[dex::CONDITION_SNOWSCAPE])
        && b.state.pokemon[m.0 as usize].species == dex::SPECIES_EISCUENOICE
    {
        b.add(LogEntry::new(
            "-activate",
            &[LogArg::Mon(m), LogArg::EffectFullName(EffectRef::Dex(ID))],
            &[],
        ));
        b.hook_state_mut(cx).payload.words[0] = 0;
        b.hook_state_mut(cx).present |= 1 << 8;
        b.forme_change(
            m,
            dex::SPECIES_EISCUE,
            EffectRef::Dex(ID),
            FormeOptions {
                permanent: true,
                message: None,
            },
        );
    }
    Relay::Undefined
}
// data/abilities.ts:1982 effect.category. Synthetic Confused/StruggleRecoil have no category.
// PRNG: none. Avoid requiring a live ActiveMove for synthetic Move-typed effects.
fn physical<L: LogSink>(b: &Battle<L>, effect: EffectRef) -> bool {
    match effect {
        EffectRef::ActiveMove(i) => b.active_move(MoveHandle(i)).category == Category::Physical,
        EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
            dex::move_data(id).category == Category::Physical
        }
        _ => false,
    }
}
