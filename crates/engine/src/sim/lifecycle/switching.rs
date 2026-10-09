//! Party permutation, initial switches and Tera action. OWNER L.
#![allow(unused_variables)]

use crate::{
    Battle,
    event::{EffectRef, Relay},
    ids::{MonId, SideId, SlotId, TypeId},
    log::LogSink,
};

/// Fixed, insertion-ordered traversal result; sim/battle.ts:1361-1379,1579-1590.
/// Entries after len are MonId::NONE. PRNG: enumeration draws nothing.
#[derive(Clone, Copy, Debug)]
pub struct PokemonList<const N: usize> {
    pub entries: [MonId; N],
    pub len: u8,
}

impl<const N: usize> Default for PokemonList<N> {
    fn default() -> Self {
        Self {
            entries: [MonId::NONE; N],
            len: 0,
        }
    }
}

impl<const N: usize> PokemonList<N> {
    /// View populated entries; battle.ts:1361-1379. PRNG: none.
    pub fn as_slice(&self) -> &[MonId] {
        &self.entries[..usize::from(self.len)]
    }
}

impl<L: LogSink> Battle<L> {
    /// Switch, transfer volatiles, permute party and enqueue runSwitch;
    /// sim/battle-actions.ts:62-160. PRNG: event ordering and insertion tie roll.
    /// Relay preserves true/false and the unreachable Gen9 pursuitfaint distinction.
    pub fn switch_in(
        &mut self,
        pokemon: MonId,
        position: u8,
        source_effect: EffectRef,
        is_drag: bool,
    ) -> Relay {
        todo!("L: switchIn")
    }

    /// Select and force a bench replacement; battle-actions.ts:162-173.
    /// PRNG: sample possibleSwitches once when nonempty, including one candidate;
    /// followed by DragOut and switchIn event draws.
    pub fn drag_in(&mut self, side: SideId, position: u8) -> bool {
        todo!("L: dragIn")
    }

    /// Batch consecutive runSwitch actions; battle-actions.ts:175-189.
    /// PRNG: speedSort(allActive,true) tie shuffles then fieldEvent SwitchIn.
    pub fn run_switch(&mut self, pokemon: MonId) {
        todo!("L: runSwitch")
    }

    /// Enumerate current party order p1 then p2; battle.ts:1361-1367.
    /// PRNG: none; stable MonId identity differs from changing party position.
    pub fn get_all_pokemon(&self) -> PokemonList<12> {
        todo!("L: getAllPokemon")
    }

    /// Enumerate p1a,p1b,p2a,p2b; battle.ts:1369-1379. PRNG: none.
    pub fn get_all_active(&self, include_fainted: bool) -> PokemonList<4> {
        todo!("L: getAllActive")
    }

    /// Count living bench options; battle.ts:1570-1572. PRNG: none.
    pub fn can_switch(&self, side: SideId) -> u8 {
        todo!("L: canSwitch")
    }

    /// Enumerate non-fainted bench in party order; battle.ts:1579-1590.
    /// PRNG: none; no options when side.pokemonLeft is zero.
    pub fn possible_switches(&self, side: SideId) -> PokemonList<6> {
        todo!("L: possibleSwitches")
    }

    /// Sample a bench option; battle.ts:1574-1577.
    /// PRNG: one sample draw when nonempty, even one candidate; none when empty.
    pub fn get_random_switchable(&mut self, side: SideId) -> Option<MonId> {
        todo!("L: getRandomSwitchable")
    }

    /// Resolve a current active slot; battle.ts:1613-1621.
    /// PRNG: none.
    pub fn get_at_slot(&self, slot: SlotId) -> Option<MonId> {
        todo!("L: getAtSlot")
    }

    /// Request eligibility for this Gen9 format; battle-actions.ts:1911-1916.
    /// PRNG: none; type zero means absent. Preserve cached per-Pokemon eligibility.
    pub fn can_terastallize(&self, pokemon: MonId) -> TypeId {
        todo!("L: canTerastallize")
    }

    /// Set Tera state and perform Ogerpon/Terapagos formes;
    /// battle-actions.ts:1918-1955. PRNG: Illusion End, forme/ability and AfterTerastallization events.
    pub fn terastallize(&mut self, pokemon: MonId) {
        todo!("L: terastallize")
    }
}
