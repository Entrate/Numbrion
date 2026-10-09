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
    /// Damage number, false, null and undefined remain distinct
    /// Ports `sim/battle-actions.ts:1572-1709`. PRNG: critical roll at :1628 when modified ratio is truthy and willCrit absent; damage randomizer plus events.
    pub fn get_damage(
        &mut self,
        source: MonId,
        target: MonId,
        move_input: DamageInput,
        options: DamageOptions,
    ) -> Relay {
        todo!("stage D: get_damage")
    }
    /// Map damage independently over surviving spread targets
    /// Ports `sim/battle-actions.ts:1137-1168`. PRNG: get_damage for each Pokemon in source order.
    pub fn get_spread_damage(
        &mut self,
        targets: Targets,
        source: MonId,
        move_handle: MoveHandle,
    ) -> TargetResults {
        todo!("stage D: get_spread_damage")
    }
    /// Exact spread/weather/crit/STAB/effectiveness/burn/modify/final clamp order
    /// Ports `sim/battle-actions.ts:1711-1835`. PRNG: one random(16) via battle.ts:2397 plus event sorts/callbacks.
    pub fn modify_damage(
        &mut self,
        base_damage: f64,
        source: MonId,
        target: MonId,
        move_handle: MoveHandle,
        options: DamageOptions,
    ) -> f64 {
        todo!("stage D: modify_damage")
    }
    /// Confusion uses its separate synthetic damage path
    /// Ports `sim/battle-actions.ts:1837-1855`. PRNG: one randomizer draw (battle.ts:2397); stat events.
    pub fn get_confusion_damage(&mut self, pokemon: MonId, base_power: f64) -> f64 {
        todo!("stage D: get_confusion_damage")
    }
}
