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
    /// Item relay or false; TakeItem veto, old item-state identity
    /// Ports `sim/pokemon.ts:1851-1866`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn take_item(&mut self, pokemon: MonId, source: Option<MonId>) -> Relay {
        todo!("stage D: take_item")
    }
    /// Install item state and Start when appropriate
    /// Ports `sim/pokemon.ts:1868-1893`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn set_item(&mut self, pokemon: MonId, item: EffectId, attribution: Attribution) -> bool {
        todo!("stage D: set_item")
    }
    /// UseItem then AfterUseItem, state flags and enditem line
    /// Ports `sim/pokemon.ts:1811-1849`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn use_item(&mut self, pokemon: MonId, attribution: Attribution) -> bool {
        todo!("stage D: use_item")
    }
    /// UseItem/TryEatItem/EatItem/Eat, End and consumption bits
    /// Ports `sim/pokemon.ts:1768-1809`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn eat_item(&mut self, pokemon: MonId, attribution: Attribution, force: bool) -> bool {
        todo!("stage D: eat_item")
    }
    /// Shared lastItem/itemUsed/ateBerry bookkeeping; does not synthesize events
    /// Ports `sim/pokemon.ts:1800-1807,1841-1847`. PRNG: none.
    pub fn consume_item(&mut self, pokemon: MonId, item: EffectId) -> () {
        todo!("stage D: consume_item")
    }
    /// Delegate set_item empty
    /// Ports `sim/pokemon.ts:1904-1906`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn clear_item(&mut self, pokemon: MonId) -> bool {
        todo!("stage D: clear_item")
    }
    /// Honor ignoringItem before matching
    /// Ports `sim/pokemon.ts:1895-1902`. PRNG: none.
    pub fn has_item(&self, pokemon: MonId, items: &[EffectId]) -> bool {
        todo!("stage D: has_item")
    }
    /// Embargo/Klutz/Magic Room and fainted/active distinctions
    /// Ports `sim/pokemon.ts:879-886`. PRNG: none.
    pub fn ignoring_item(&self, pokemon: MonId) -> bool {
        todo!("stage D: ignoring_item")
    }
    /// Old-ability relay/false/null; End/Start and suppression restrictions
    /// Ports `sim/pokemon.ts:1908-1950`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn set_ability(
        &mut self,
        pokemon: MonId,
        ability: EffectId,
        attribution: Attribution,
        is_from_forme_change: bool,
        is_transform: bool,
    ) -> Relay {
        todo!("stage D: set_ability")
    }
    /// Set empty ability
    /// Ports `sim/pokemon.ts:1961-1963`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn clear_ability(&mut self, pokemon: MonId) -> Relay {
        todo!("stage D: clear_ability")
    }
    /// Honor ignoringAbility before matching
    /// Ports `sim/pokemon.ts:1952-1959`. PRNG: none.
    pub fn has_ability(&self, pokemon: MonId, abilities: &[EffectId]) -> bool {
        todo!("stage D: has_ability")
    }
    /// Gastro Acid and Neutralizing Gas exceptions independent of Mold Breaker
    /// Ports `sim/pokemon.ts:858-877`. PRNG: none.
    pub fn ignoring_ability(&self, pokemon: MonId) -> bool {
        todo!("stage D: ignoring_ability")
    }
}
