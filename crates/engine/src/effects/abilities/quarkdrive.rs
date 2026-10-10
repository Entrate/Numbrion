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
pub const ID: EffectId = dex::ABILITY_QUARKDRIVE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_QUARKDRIVE_ONSTART,
    dex::HOOK_ABILITY_QUARKDRIVE_ONTERRAINCHANGE,
    dex::HOOK_ABILITY_QUARKDRIVE_ONEND,
];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3653. PRNG: nested TerrainChange event only.
        dex::HOOK_ABILITY_QUARKDRIVE_ONSTART => {
            b.single_event(
                EventId::TerrainChange,
                EffectRef::Dex(ID),
                Some(cx.state),
                b.event_arg(cx, 0),
                EventArg::Undefined,
                EffectRef::None,
                Relay::Undefined,
                None,
            );
            Relay::Undefined
        }
        // data/abilities.ts:3656. PRNG: volatile Start/End and environment events only.
        dex::HOOK_ABILITY_QUARKDRIVE_ONTERRAINCHANGE => {
            let m = mon(b, cx, 0);
            if b.is_terrain(&[dex::CONDITION_ELECTRICTERRAIN], None) {
                b.add_volatile(m, dex::CONDITION_QUARKDRIVE, Attribution::DEFAULT, None);
            } else {
                let from = b
                    .get_volatile(m, dex::CONDITION_QUARKDRIVE)
                    .is_some_and(|c| b.state.effects.cells[c.0 as usize].payload.words[0] != 0);
                if !from {
                    b.remove_volatile(m, dex::CONDITION_QUARKDRIVE);
                }
            }
            Relay::Undefined
        }
        // data/abilities.ts:3664. PRNG: none. JS delete skips condition End, including at zero HP.
        dex::HOOK_ABILITY_QUARKDRIVE_ONEND => {
            let m = mon(b, cx, 0);
            b.erase_volatile(m, dex::CONDITION_QUARKDRIVE);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(m), LogArg::Text("Quark Drive")],
                &[LogTag::Bare("silent")],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected quarkdrive hook"),
    }
}
