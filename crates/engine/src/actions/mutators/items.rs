//! Item/ability queries ported from pinned Showdown; lifecycle mutators remain explicit stubs.
#![allow(unused_variables, unused_imports)]
use crate::{
    Battle,
    actions::*,
    dex::{self, ImmunityId},
    event::{EffectRef, Relay},
    ids::*,
    log::LogSink,
    state::{
        CellId, mon_flags, present,
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
    /// Match the item first, then honor ignoringItem
    /// Ports `sim/pokemon.ts:1895-1902`. PRNG: none.
    pub fn has_item(&self, pokemon: MonId, items: &[EffectId]) -> bool {
        items.contains(&self.state.pokemon[pokemon.0 as usize].item) && !self.ignoring_item(pokemon)
    }
    /// Embargo/Klutz/Magic Room and fainted/active distinctions
    /// Ports `sim/pokemon.ts:879-886`. PRNG: none.
    pub fn ignoring_item(&self, pokemon: MonId) -> bool {
        let mon = &self.state.pokemon[pokemon.0 as usize];
        // pokemon.ts:880: Primal Orbs precede even the inactive check. Read the
        // generated property because ItemData deliberately omits this rare field.
        if mon.item != EffectId::NONE
            && dex::effect(mon.item).data.get(dex::FIELD_ISPRIMALORB)
                == Some(dex::DataValue::Bool(true))
        {
            return false;
        }
        if mon.flags & mon_flags::ACTIVE == 0 {
            return true;
        }
        if self.query_has_volatile(pokemon, "embargo")
            || self.state.field.pseudo_weather.cells[..self.state.field.pseudo_weather.len as usize]
                .iter()
                .any(|c| query_effect_is(self.state.effects.cells[c.0 as usize].id, "magicroom"))
        {
            return true;
        }
        let ignore_klutz = mon.item != EffectId::NONE
            && dex::ITEMS[(mon.item.0 - dex::ITEM_START) as usize].ignore_klutz;
        !ignore_klutz && self.query_has_ability(pokemon, "klutz")
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
    /// Match the ability first, then honor ignoringAbility
    /// Ports `sim/pokemon.ts:1952-1959`. PRNG: none.
    pub fn has_ability(&self, pokemon: MonId, abilities: &[EffectId]) -> bool {
        abilities.contains(&self.state.pokemon[pokemon.0 as usize].ability)
            && !self.ignoring_ability(pokemon)
    }
    /// Gastro Acid and Neutralizing Gas exceptions independent of Mold Breaker
    /// Ports `sim/pokemon.ts:858-877`. PRNG: none.
    pub fn ignoring_ability(&self, pokemon: MonId) -> bool {
        let mon = &self.state.pokemon[pokemon.0 as usize];
        if mon.flags & mon_flags::ACTIVE == 0 {
            return true;
        }
        let flags = if mon.ability == EffectId::NONE {
            0
        } else {
            dex::ABILITIES[(mon.ability.0 - dex::ABILITY_START) as usize].flags
        };
        // Transformation suppression deliberately precedes cantsuppress.
        if flags & dex::FLAG_NOTRANSFORM != 0 && mon.flags & mon_flags::TRANSFORMED != 0 {
            return true;
        }
        if flags & dex::FLAG_CANTSUPPRESS != 0 {
            return false;
        }
        if self.query_has_volatile(pokemon, "gastroacid") {
            return true;
        }
        if self.query_has_item(pokemon, "abilityshield")
            || query_effect_is(mon.ability, "neutralizinggas")
        {
            return false;
        }
        // Literal getAllActive() order/filter. Never recurse through hasAbility
        // on a gas holder; abilityState.ending is a shared truthiness flag.
        for side in &self.state.sides {
            for &active in &side.active {
                if active == MonId::NONE {
                    continue;
                }
                let holder = &self.state.pokemon[active.0 as usize];
                if holder.flags & mon_flags::FAINTED != 0 {
                    continue;
                }
                if query_effect_is(holder.ability, "neutralizinggas")
                    && !self.query_has_volatile(active, "gastroacid")
                    && holder.flags & mon_flags::TRANSFORMED == 0
                    && self.state.effects.cells[holder.ability_state.0 as usize].present
                        & present::ENDING
                        == 0
                    && !self.query_has_volatile(pokemon, "commanding")
                {
                    return true;
                }
            }
        }
        false
    }
}

// Static Dex keys let format-excluded source branches remain explicit without
// fabricating IDs for effects absent from this generated scope. NONE never
// matches a named ability/item/condition. These queries allocate and draw nothing.
fn query_effect_is(effect: EffectId, key: &str) -> bool {
    effect != EffectId::NONE && dex::effect(effect).key == key
}
impl<L: LogSink> Battle<L> {
    pub(crate) fn query_has_volatile(&self, pokemon: MonId, key: &str) -> bool {
        let list = &self.state.pokemon[pokemon.0 as usize].volatiles;
        list.cells[..list.len as usize]
            .iter()
            .any(|c| query_effect_is(self.state.effects.cells[c.0 as usize].id, key))
    }
    pub(crate) fn query_has_ability(&self, pokemon: MonId, key: &str) -> bool {
        query_effect_is(self.state.pokemon[pokemon.0 as usize].ability, key)
            && !self.ignoring_ability(pokemon)
    }
    pub(crate) fn query_has_item(&self, pokemon: MonId, key: &str) -> bool {
        query_effect_is(self.state.pokemon[pokemon.0 as usize].item, key)
            && !self.ignoring_item(pokemon)
    }
}
