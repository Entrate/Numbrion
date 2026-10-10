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
pub const ID: EffectId = dex::ABILITY_DISGUISE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_DISGUISE_ONDAMAGE,
    dex::HOOK_ABILITY_DISGUISE_ONCRITICALHIT,
    dex::HOOK_ABILITY_DISGUISE_ONEFFECTIVENESS,
    dex::HOOK_ABILITY_DISGUISE_ONUPDATE,
];
// Payload: 1 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:972-978. PRNG: none; numeric zero absorbs a hit.
        // word0=busted, bit8=present (true on damage, false on Ice Face restoration).
        dex::HOOK_ABILITY_DISGUISE_ONDAMAGE => {
            let target = mon(b, cx, 1);
            let effect = effect_at(b, cx, 3);
            if effect_is_move(b, effect)
                && b.state.pokemon[target.0 as usize].species == dex::SPECIES_MIMIKYU
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
        // data/abilities.ts:979-989. PRNG: runImmunity events only.
        dex::HOOK_ABILITY_DISGUISE_ONCRITICALHIT => shield(b, cx, 2, false),
        // data/abilities.ts:990-1001. PRNG: runImmunity events only.
        dex::HOOK_ABILITY_DISGUISE_ONEFFECTIVENESS => shield(b, cx, 3, true),
        // data/abilities.ts:1002-1008. PRNG: forme/damage events only.
        dex::HOOK_ABILITY_DISGUISE_ONUPDATE => {
            let m = mon(b, cx, 0);
            if b.state.pokemon[m.0 as usize].species == dex::SPECIES_MIMIKYU
                && b.hook_state(cx).payload.words[0] != 0
            {
                b.forme_change(
                    m,
                    dex::SPECIES_MIMIKYUBUSTED,
                    EffectRef::Dex(ID),
                    FormeOptions {
                        permanent: true,
                        message: None,
                    },
                );
                b.damage(
                    b.state.pokemon[m.0 as usize].max_hp as f64 / 8.,
                    Some(m),
                    Attribution::from_move(m, EffectRef::Dex(dex::SPECIES_MIMIKYUBUSTED)),
                );
            }
            Relay::Undefined
        }
        _ => panic!("unexpected disguise hook"),
    }
}

// data/abilities.ts:980, 991. PRNG: runImmunity events only.
fn shield<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, index: usize, effectiveness: bool) -> Relay {
    let Some(target) = mon_opt(b, cx, if effectiveness { 1 } else { 0 }) else {
        return Relay::Undefined;
    };
    let h = MoveHandle(move_arg(b, cx, index));
    let mv = *b.active_move(h);
    if b.state.pokemon[target.0 as usize].species != dex::SPECIES_MIMIKYU
        || (effectiveness && mv.category == Category::Status)
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
