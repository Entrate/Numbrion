//! Pokemon read/contact helpers ported from pinned Showdown; other mutators remain explicit stubs.
#![allow(unused_variables, unused_imports)]
use crate::{
    Battle,
    actions::*,
    dex::{self, ImmunityId},
    event::{EffectRef, EventArg, Relay, RunEventOptions},
    ids::*,
    log::{LogArg, LogEntry, LogSink},
    state::{
        CellId,
        scratch::{HitData, OrderedBoosts},
    },
};
impl<L: LogSink> Battle<L> {
    /// ModifySpecies relay, recalculated stats/types/weight and baseMaxhp
    /// Ports `sim/pokemon.ts:1387-1425`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn set_species(
        &mut self,
        pokemon: MonId,
        species: EffectId,
        source_effect: EffectRef,
        is_transform: bool,
    ) -> Relay {
        todo!("stage D: set_species")
    }
    /// Permanent/base details/ability and temporary forme logging
    /// Ports `sim/pokemon.ts:1427-1506`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn forme_change(
        &mut self,
        pokemon: MonId,
        species: EffectId,
        source_effect: EffectRef,
        options: FormeOptions,
    ) -> bool {
        todo!("stage D: forme_change")
    }
    /// Virtual move PP/types/boosts/ability; preserve real base slot ownership
    /// Ports `sim/pokemon.ts:1270-1385`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn transform_into(&mut self, pokemon: MonId, target: MonId, effect: EffectRef) -> bool {
        todo!("stage D: transform_into")
    }
    /// Baton Pass/Shed Tail filtered boost+volatile transfer and relinking
    /// Ports `sim/pokemon.ts:1246-1268`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn copy_volatile_from(
        &mut self,
        pokemon: MonId,
        source: MonId,
        switch_effect: EffectId,
    ) -> () {
        todo!("stage D: copy_volatile_from")
    }
    /// Reset Transform, move slots, boosts and links; preserve base data
    /// Ports `sim/pokemon.ts:1508-1567`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn clear_volatile(&mut self, pokemon: MonId, include_switch_flags: bool) -> () {
        todo!("stage D: clear_volatile")
    }
    /// Tera/Multitype/RKS restrictions and apparent type state
    /// Ports `sim/pokemon.ts:2109-2130`. PRNG: none.
    pub fn set_type(&mut self, pokemon: MonId, types: Types, enforce: bool) -> bool {
        todo!("stage D: set_type")
    }
    /// Added type and knownType flag
    /// Ports `sim/pokemon.ts:2132-2136`. PRNG: none.
    pub fn add_type(&mut self, pokemon: MonId, move_type: TypeId) -> bool {
        todo!("stage D: add_type")
    }
    /// Protective Pads before contact flags; Long Reach acts through ModifyMove
    /// Ports `sim/battle.ts:1293-1302`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn check_move_makes_contact(
        &mut self,
        move_handle: MoveHandle,
        attacker: MonId,
        defender: MonId,
        announce_pads: bool,
    ) -> bool {
        let contact = self.active_move(move_handle).flags & dex::FLAG_CONTACT != 0;
        if contact && self.query_has_item(attacker, "protectivepads") {
            if announce_pads {
                self.add(LogEntry::new(
                    "-activate",
                    &[
                        LogArg::Mon(defender),
                        LogArg::EffectFullName(self.scratch.current_effect),
                    ],
                    &[],
                ));
                self.add(LogEntry::new(
                    "-activate",
                    &[LogArg::Mon(attacker), LogArg::Text("item: Protective Pads")],
                    &[],
                ));
            }
            return false;
        }
        contact
    }
    /// Unseen Fist and protect-blocking flags
    /// Ports `sim/battle.ts:1304-1313`. PRNG: none directly; HitProtect callbacks may draw.
    pub fn check_move_bypasses_protect(
        &mut self,
        move_handle: MoveHandle,
        attacker: MonId,
        defender: MonId,
        block_status: bool,
    ) -> bool {
        let mv = self.active_move(move_handle);
        let check_protect = (mv.category != dex::Category::Status || block_status)
            && mv.flags & dex::FLAG_PROTECT != 0;
        // Pinned Gen 9 random doubles has no Z/Max moves. Their mutable
        // isZOrMaxPowered property is outside the frozen ActiveMove interface.
        if check_protect
            && self
                .run_event(
                    EventId::HitProtect,
                    EventArg::Holder(Holder::mon(attacker)),
                    EventArg::Holder(Holder::mon(defender)),
                    EffectRef::ActiveMove(move_handle.0),
                    Relay::Undefined,
                    RunEventOptions::default(),
                )
                .truthy()
        {
            return false;
        }
        true
    }
    /// Exchange abilities while preserving required End/Start order
    /// Ports `sim/battle.ts:1315-1349`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn skill_swap(&mut self, source: MonId, target: MonId) -> bool {
        todo!("stage D: skill_swap")
    }
}
impl<L: LogSink> Battle<L> {
    /// Ports sim/pokemon.ts:1498-1506. PRNG: none directly; stat helpers may dispatch.
    pub fn update_max_hp(&mut self, pokemon: MonId) -> () {
        todo!("stage D: update_max_hp")
    }
    /// Ports sim/pokemon.ts:1625-1637. PRNG: none. Preserve hidden versus true disable.
    pub fn disable_move(
        &mut self,
        pokemon: MonId,
        move_id: EffectId,
        hidden: bool,
        source_effect: EffectRef,
    ) -> () {
        todo!("stage D: disable_move")
    }
    /// Ports sim/pokemon.ts:1614-1623. PRNG: none. Move ID or false sentinel.
    pub fn has_move(&self, pokemon: MonId, move_id: EffectId) -> Relay {
        if self.get_move_data(pokemon, move_id).is_some() {
            Relay::Move(move_id)
        } else {
            Relay::Bool(false)
        }
    }
    /// Ports sim/pokemon.ts:690-704. PRNG: none. Read current base/virtual slot.
    pub fn get_move_data(
        &self,
        pokemon: MonId,
        move_id: EffectId,
    ) -> Option<&crate::state::MoveSlot> {
        self.state.pokemon[pokemon.0 as usize]
            .move_slots()
            .iter()
            .find(|slot| slot.id == move_id)
    }
    /// Ports sim/pokemon.ts:1210-1219. PRNG: none. Clamp sparse deltas without mutation.
    pub fn get_capped_boost(&self, pokemon: MonId, boosts: OrderedBoosts) -> OrderedBoosts {
        let current = &self.state.pokemon[pokemon.0 as usize].boosts;
        let mut capped = OrderedBoosts::default();
        for &stat in &boosts.order[..boosts.len as usize] {
            let i = stat as usize;
            let delta = boosts.values[i];
            if delta == 0 {
                continue;
            }
            capped.values[i] =
                (i16::from(current[i]) + i16::from(delta)).clamp(-6, 6) as i8 - current[i];
            capped.order[capped.len as usize] = stat;
            capped.len += 1;
            capped.present |= 1 << i;
        }
        capped
    }
    /// Ports sim/pokemon.ts:1201-1208. PRNG: none. Sum positive boost stages.
    pub fn positive_boosts(&self, pokemon: MonId) -> u8 {
        self.state.pokemon[pokemon.0 as usize]
            .boosts
            .iter()
            .filter(|&&stage| stage > 0)
            .map(|&stage| stage as u8)
            .sum()
    }
}
impl<L: LogSink> Battle<L> {
    /// Ports sim/pokemon.ts:748-752. PRNG: none; baseMaxhp==maxhp in this format.
    pub fn get_undynamaxed_hp(&self, pokemon: MonId, hp: Option<f64>) -> f64 {
        // JS amount || this.hp, including zero and NaN. Dynamax is outside
        // this format: baseMaxhp and maxhp are identical and no cell exists.
        hp.filter(|n| *n != 0.0 && !n.is_nan())
            .unwrap_or(f64::from(self.state.pokemon[pokemon.0 as usize].hp))
    }
    /// Ports sim/pokemon.ts:1965-1967. PRNG: none. Immutable input nature; neutral if absent.
    pub fn get_nature(&self, pokemon: MonId) -> Option<&'static crate::dex::NatureData> {
        let set = &self.teams.sides[pokemon.side().0 as usize].sets[(pokemon.0 % 6) as usize];
        (set.nature >= 0).then(|| &dex::NATURES[set.nature as usize])
    }
    /// Engine projection of queued faint effects (pokemon.ts:1581-1593,battle.ts:2539).
    /// PRNG: none. ActiveMove projects to its immutable ID after runtime users finish.
    pub fn freeze_effect(&self, effect: EffectRef) -> EffectToken {
        todo!("stage D: freeze_effect")
    }
}

#[cfg(test)]
#[path = "queries/tests.rs"]
mod query_tests;
