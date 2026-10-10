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
pub const ID: EffectId = dex::SPECIES_ARCEUS;
pub const HOOKS: &[HookId] = &[dex::HOOK_SPECIES_ARCEUS_ONTYPE];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:853-863; sim/dex-species.ts:529-536 inherits the base view.
        // PRNG: none. Same canonical callback for every Arceus view, with distinct HookIds.
        dex::HOOK_SPECIES_ARCEUS_ONTYPE => on_type(b, cx),
        _ => panic!("unexpected arceus hook"),
    }
}

// data/conditions.ts:853-863. PRNG: none. Raw ability/item: no suppression queries.
pub(super) fn on_type<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon(b, cx, 1);
    let p = b.state.pokemon[m.0 as usize];
    if p.flags & mon_flags::TRANSFORMED != 0 || p.ability != dex::ABILITY_MULTITYPE {
        return match b.event_arg(cx, 0) {
            EventArg::Relay(r) => r,
            _ => panic!("Type requires array relay"),
        };
    }
    let plate = if p.item == EffectId::NONE {
        TypeId::NONE
    } else {
        dex::ITEMS[(p.item.0 - dex::ITEM_START) as usize].on_plate
    };
    let ty = if plate == TypeId::NONE {
        dex::TYPE_NORMAL
    } else {
        plate
    };
    Relay::Types(b.stash_types(Types {
        values: [ty, TypeId::NONE, TypeId::NONE],
        len: 1,
    }))
}
