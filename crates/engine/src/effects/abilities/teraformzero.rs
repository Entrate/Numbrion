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
pub const ID: EffectId = dex::ABILITY_TERAFORMZERO;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_TERAFORMZERO_ONAFTERTERASTALLIZATION];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4946-4953. PRNG: clearWeather/clearTerrain events only, in that order.
        dex::HOOK_ABILITY_TERAFORMZERO_ONAFTERTERASTALLIZATION => {
            let m = mon(b, cx, 0);
            if b.state.pokemon[m.0 as usize].base_species != dex::SPECIES_TERAPAGOSSTELLAR {
                return Relay::Undefined;
            }
            if b.state.effects.cells[b.state.field.weather.0 as usize].id != EffectId::NONE
                || b.state.effects.cells[b.state.field.terrain.0 as usize].id != EffectId::NONE
            {
                b.add(LogEntry::new(
                    "-ability",
                    &[LogArg::Mon(m), LogArg::Effect(EffectRef::Dex(ID))],
                    &[],
                ));
                b.clear_weather();
                b.clear_terrain();
            }
            Relay::Undefined
        }
        _ => panic!("unexpected teraformzero hook"),
    }
}
