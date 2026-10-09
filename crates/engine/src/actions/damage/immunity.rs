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
    /// Run per-type Effectiveness hooks then sum modifiers
    /// Ports `sim/pokemon.ts:2208-2234`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn run_effectiveness(&mut self, target: MonId, move_handle: MoveHandle) -> i8 {
        todo!("stage D: run_effectiveness")
    }
    /// Type immunity including groundedness and ignore-immunity events
    /// Ports `sim/pokemon.ts:2236-2267`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn run_immunity(
        &mut self,
        target: MonId,
        source: ImmunitySource,
        message: ImmunityMessage,
    ) -> bool {
        todo!("stage D: run_immunity")
    }
    /// Status/powder/sandstorm immunity has a separate name space
    /// Ports `sim/pokemon.ts:2269-2294`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn run_status_immunity(
        &mut self,
        target: MonId,
        immunity: ImmunityId,
        message: ImmunityMessage,
    ) -> bool {
        todo!("stage D: run_status_immunity")
    }
    /// Includes added type, Tera and Type events
    /// Ports `sim/pokemon.ts:2138-2146`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn get_types(&mut self, pokemon: MonId, exclude_added: bool, pre_tera: bool) -> Types {
        todo!("stage D: get_types")
    }
    /// Null distinguishes Levitate from explicit ungrounded false
    /// Ports `sim/pokemon.ts:2148-2160`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn is_grounded(&mut self, pokemon: MonId, negate_immunity: bool) -> Relay {
        todo!("stage D: is_grounded")
    }
    /// Read charge/commanding/semi-invulnerability state
    /// Ports `sim/pokemon.ts:2162-2188`. PRNG: none.
    pub fn is_semi_invulnerable(&self, pokemon: MonId) -> bool {
        todo!("stage D: is_semi_invulnerable")
    }
    /// TryWeather can suppress weather separately per Pokemon
    /// Ports `sim/pokemon.ts:2190-2201`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn effective_weather(&mut self, pokemon: MonId) -> EffectId {
        todo!("stage D: effective_weather")
    }
}
impl<L: LogSink> Battle<L> {
    /// Ports sim/pokemon.ts:1567-1579. PRNG: Type handlers/events; preserve repeated queries.
    pub fn has_type(&mut self, pokemon: MonId, types: &[TypeId]) -> bool {
        todo!("stage D: has_type")
    }
}
