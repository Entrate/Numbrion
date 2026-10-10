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
pub const ID: EffectId = dex::SPECIES_ZACIAN;
pub const HOOKS: &[HookId] = &[dex::HOOK_SPECIES_ZACIAN_ONBATTLESTART];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:882-909; sim/dex-species.ts:529-536 inherits base view.
        // PRNG: ModifySpecies/setAbility events only. Crowned views route the same body.
        dex::HOOK_SPECIES_ZACIAN_ONBATTLESTART => on_start(b, cx),
        _ => panic!("unexpected zacian hook"),
    }
}

// data/conditions.ts:882-909. PRNG: nested events only; no forme-change protocol lines.
pub(super) fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon(b, cx, 0);
    if b.state.pokemon[m.0 as usize].item != dex::ITEM_RUSTEDSWORD {
        return Relay::Undefined;
    }
    let Relay::Species(actual) = b.set_species(
        m,
        dex::SPECIES_ZACIANCROWNED,
        b.scratch.current_effect,
        false,
    ) else {
        return Relay::Undefined;
    };
    b.state.pokemon[m.0 as usize].base_species = dex::SPECIES_ZACIANCROWNED;
    // Details are represented structurally by baseSpecies/current species in the log view.
    b.set_ability(
        m,
        dex::species(actual).abilities[0].id,
        Attribution::NONE,
        true,
        false,
    );
    b.state.pokemon[m.0 as usize].base_ability = b.state.pokemon[m.0 as usize].ability;
    let p = b.state.pokemon[m.0 as usize];
    if let Some(i) = p.base_move_slots[..p.move_count as usize]
        .iter()
        .position(|slot| slot.id == dex::MOVE_IRONHEAD)
    {
        // pokemon.ts:361-373: explicit packed teams give Iron Head three PP Ups.
        // No calculatePP core signature exists; do not duplicate universal PP calculation here.
        let pp = calculate_pp(b, dex::MOVE_BEHEMOTHBLADE, 3);
        b.state.pokemon[m.0 as usize].base_move_slots[i] = crate::state::MoveSlot {
            id: dex::MOVE_BEHEMOTHBLADE,
            pp,
            max_pp: pp,
            target: dex::move_data(dex::MOVE_BEHEMOTHBLADE).target,
            ..Default::default()
        };
        b.state.pokemon[m.0 as usize].virtual_move_count = 0;
    }
    Relay::Undefined
}
// sim/battle.ts:2378-2383; data/conditions.ts:895. PRNG: none.
fn calculate_pp<L: LogSink>(_b: &Battle<L>, _move_id: EffectId, _pp_ups: u8) -> u8 {
    todo!("needs Battle::calculate_pp(move_id, pp_ups): crowned Iron Head replacement PP")
}
