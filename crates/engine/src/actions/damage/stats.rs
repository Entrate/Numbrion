//! OWNER D. Pinned Showdown ports; all behavior remains an explicit stub.
#![allow(unused_variables, unused_imports)]
use crate::{
    Battle,
    actions::*,
    dex::ImmunityId,
    event::{EffectRef, Relay},
    ids::*,
    log::LogSink,
    state::{
        CellId,
        scratch::{HitData, OrderedBoosts},
    },
};
impl<L: LogSink> Battle<L> {
    /// Wonder Room then sparse ModifyBoost and explicit modifier
    /// Ports `sim/pokemon.ts:560-594`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn calculate_stat(
        &mut self,
        pokemon: MonId,
        stat: Stat,
        boost: i8,
        modifier: Option<f64>,
        stat_user: Option<MonId>,
    ) -> f64 {
        todo!("stage D: calculate_stat")
    }
    /// Mutable event dispatch despite being a stat query
    /// Ports `sim/pokemon.ts:596-639`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn get_stat(&mut self, pokemon: MonId, stat: Stat, options: StatOptions) -> f64 {
        todo!("stage D: get_stat")
    }
    /// Trick Room reverses and wraps modified speed
    /// Ports `sim/pokemon.ts:641-657`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn pokemon_action_speed(&mut self, pokemon: MonId) -> u16 {
        todo!("stage D: pokemon_action_speed")
    }
    /// Refresh cached speed via get_stat
    /// Ports `sim/pokemon.ts:556-558`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn update_pokemon_speed(&mut self, pokemon: MonId) -> () {
        todo!("stage D: update_pokemon_speed")
    }
    /// Hit metadata belongs to this move-use scratch frame
    /// Ports `sim/pokemon.ts:706-711`. PRNG: none.
    pub fn move_hit_data(&mut self, target: MonId, move_handle: MoveHandle) -> &mut HitData {
        todo!("stage D: move_hit_data")
    }
}
impl<L: LogSink> Battle<L> {
    /// Ports sim/pokemon.ts:656-668. PRNG: getStat events and callbacks; repeat
    /// getStat when a new maximum is found, exactly as TS (do not memoize).
    pub fn get_best_stat(&mut self, pokemon: MonId, options: StatOptions) -> Stat {
        todo!("stage D: get_best_stat")
    }
    /// Ports sim/pokemon.ts:685-688. PRNG: ModifyWeight handler sorting/callbacks.
    pub fn get_weight(&mut self, pokemon: MonId) -> f64 {
        todo!("stage D: get_weight")
    }
}
