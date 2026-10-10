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
pub const ID: EffectId = dex::CONDITION_QUARKDRIVE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_QUARKDRIVE_ONSTART,
    dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYATK,
    dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYDEF,
    dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYSPA,
    dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYSPD,
    dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYSPE,
    dex::HOOK_CONDITION_QUARKDRIVE_ONEND,
];
// Payload: 2 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 2;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3669-3678. PRNG: getBestStat(false,true) has no modified-stat event draws.
        // word0=fromBooster (bit8=present); word1=bestStat as Stat (bit9=present).
        dex::HOOK_CONDITION_QUARKDRIVE_ONSTART => {
            let m = mon(b, cx, 0);
            let effect = effect_at(b, cx, 2);
            if b.event_effect_id(effect) == dex::ITEM_BOOSTERENERGY {
                b.hook_state_mut(cx).payload.words[0] = 1;
                b.hook_state_mut(cx).present |= 1 << 8;
                b.add(LogEntry::new(
                    "-activate",
                    &[
                        LogArg::Mon(m),
                        LogArg::EffectFullName(EffectRef::Dex(dex::ABILITY_QUARKDRIVE)),
                    ],
                    &[LogTag::Bare("fromitem")],
                ));
            } else {
                b.add(LogEntry::new(
                    "-activate",
                    &[
                        LogArg::Mon(m),
                        LogArg::EffectFullName(EffectRef::Dex(dex::ABILITY_QUARKDRIVE)),
                    ],
                    &[],
                ));
            }
            let stat = b.get_best_stat(
                m,
                StatOptions {
                    unboosted: false,
                    unmodified: true,
                },
            );
            b.hook_state_mut(cx).payload.words[1] = stat as u32;
            b.hook_state_mut(cx).present |= 1 << 9;
            let suffix = match stat {
                Stat::Atk => "atk",
                Stat::Def => "def",
                Stat::SpA => "spa",
                Stat::SpD => "spd",
                Stat::Spe => "spe",
            };
            let parts = [LogArg::Text("quarkdrive"), LogArg::Text(suffix)];
            b.add(LogEntry::new(
                "-start",
                &[LogArg::Mon(m), LogArg::Parts(&parts)],
                &[],
            ));
            Relay::Undefined
        }
        // data/abilities.ts:3680-3684. PRNG: none. Suppression is rechecked for each stat query.
        dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYATK => {
            let m = mon(b, cx, 1);
            let state = b.hook_state(cx);
            if state.present & (1 << 9) != 0
                && state.payload.words[1] == Stat::Atk as u32
                && !b.ignoring_ability(m)
            {
                b.chain_modify(5325., 4096.);
            }
            Relay::Undefined
        }
        // data/abilities.ts:3686-3690. PRNG: none. Suppression is rechecked for each stat query.
        dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYDEF => {
            let m = mon(b, cx, 1);
            let state = b.hook_state(cx);
            if state.present & (1 << 9) != 0
                && state.payload.words[1] == Stat::Def as u32
                && !b.ignoring_ability(m)
            {
                b.chain_modify(5325., 4096.);
            }
            Relay::Undefined
        }
        // data/abilities.ts:3692-3696. PRNG: none. Suppression is rechecked for each stat query.
        dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYSPA => {
            let m = mon(b, cx, 1);
            let state = b.hook_state(cx);
            if state.present & (1 << 9) != 0
                && state.payload.words[1] == Stat::SpA as u32
                && !b.ignoring_ability(m)
            {
                b.chain_modify(5325., 4096.);
            }
            Relay::Undefined
        }
        // data/abilities.ts:3698-3702. PRNG: none. Suppression is rechecked for each stat query.
        dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYSPD => {
            let m = mon(b, cx, 1);
            let state = b.hook_state(cx);
            if state.present & (1 << 9) != 0
                && state.payload.words[1] == Stat::SpD as u32
                && !b.ignoring_ability(m)
            {
                b.chain_modify(5325., 4096.);
            }
            Relay::Undefined
        }
        // data/abilities.ts:3703-3707. PRNG: none. Suppression is rechecked for each stat query.
        dex::HOOK_CONDITION_QUARKDRIVE_ONMODIFYSPE => {
            let m = mon(b, cx, 1);
            let state = b.hook_state(cx);
            if state.present & (1 << 9) != 0
                && state.payload.words[1] == Stat::Spe as u32
                && !b.ignoring_ability(m)
            {
                b.chain_modify(3., 2.);
            }
            Relay::Undefined
        }
        // data/abilities.ts:3708-3710. PRNG: none.
        dex::HOOK_CONDITION_QUARKDRIVE_ONEND => {
            let m = mon(b, cx, 0);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(m), LogArg::Text("Quark Drive")],
                &[],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected quarkdrive hook"),
    }
}
