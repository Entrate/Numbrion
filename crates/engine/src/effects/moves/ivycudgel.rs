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
pub const ID: EffectId = dex::MOVE_IVYCUDGEL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_IVYCUDGEL_ONPREPAREHIT,
    dex::HOOK_MOVE_IVYCUDGEL_ONMODIFYTYPE,
];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:9768-9772. PRNG: none; structured Parts keep one animation tag.
        dex::HOOK_MOVE_IVYCUDGEL_ONPREPAREHIT => {
            let h = MoveHandle(move_arg(b, cx, 2));
            let ty = b.active_move(h).move_type;
            if ty != dex::TYPE_GRASS {
                let parts = [LogArg::Text("Ivy Cudgel "), LogArg::Type(ty)];
                b.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
                    "anim",
                    LogArg::Parts(&parts),
                )));
            }
            Relay::Undefined
        }
        // data/moves.ts:9773-9785. PRNG: none. No default assignment for unrelated species.
        dex::HOOK_MOVE_IVYCUDGEL_ONMODIFYTYPE => {
            let h = MoveHandle(move_arg(b, cx, 0));
            let m = mon(b, cx, 1);
            let ty = match b.state.pokemon[m.0 as usize].species {
                dex::SPECIES_OGERPONWELLSPRING | dex::SPECIES_OGERPONWELLSPRINGTERA => {
                    Some(dex::TYPE_WATER)
                }
                dex::SPECIES_OGERPONHEARTHFLAME | dex::SPECIES_OGERPONHEARTHFLAMETERA => {
                    Some(dex::TYPE_FIRE)
                }
                dex::SPECIES_OGERPONCORNERSTONE | dex::SPECIES_OGERPONCORNERSTONETERA => {
                    Some(dex::TYPE_ROCK)
                }
                _ => None,
            };
            if let Some(ty) = ty {
                b.active_move_mut(h).move_type = ty;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected ivycudgel hook"),
    }
}
