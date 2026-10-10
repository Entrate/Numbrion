//! Pinned Showdown state mutation ports.
#![allow(unused_variables, unused_imports)]
use super::common::{mon_arg, number};
use crate::{
    Battle,
    actions::*,
    dex::{self, ImmunityId},
    event::{EffectRef, EventArg, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{
        CellId, present,
        scratch::{HitData, OrderedBoosts},
    },
};
impl<L: LogSink> Battle<L> {
    /// Restart or duration+SideStart installation
    /// Ports `sim/side.ts:413-441`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn add_side_condition(
        &mut self,
        side: SideId,
        status: EffectId,
        attribution: Attribution,
    ) -> Relay {
        let a = self.side_condition_attribution(side, attribution);
        let status = self.condition_id(status);
        let effect = self.condition_ref(status);
        let target = EventArg::Holder(Holder::side(side));
        if let Some(c) = self.get_side_condition_data(side, status) {
            if self
                .event_hook(effect, EventId::SideRestart, dex::HookRel::On)
                .is_none()
            {
                return Relay::Bool(false);
            }
            return self.mutation_single(EventId::SideRestart, effect, c, target, a);
        }
        let c = self.mutation_cell(Holder::side(side), Holder::side(side), status);
        self.state.sides[side.0 as usize].conditions.push(c);
        self.condition_source(c, a, true, false);
        self.condition_duration(
            c,
            effect,
            mon_arg(Some(self.state.sides[side.0 as usize].active[0])),
            a,
        );
        if !self
            .mutation_single(EventId::SideStart, effect, c, target, a)
            .truthy()
        {
            self.erase_side_condition(side, status);
            return Relay::Bool(false);
        }
        self.mutation_event(
            EventId::SideConditionStart,
            target,
            Attribution { effect, ..a },
            Relay::Undefined,
        );
        Relay::Bool(true)
    }
    /// Condition lookup
    /// Ports `sim/side.ts:443-447`. PRNG: none.
    pub fn get_side_condition(&self, side: SideId, status: EffectId) -> Option<EffectId> {
        self.get_side_condition_data(side, status)
            .map(|c| self.state.effects.cells[c.0 as usize].id)
    }
    /// Return condition state, not an active move object
    /// Ports `sim/side.ts:449-454`. PRNG: none.
    pub fn get_side_condition_data(&self, side: SideId, status: EffectId) -> Option<CellId> {
        let status = self.condition_id(status);
        self.state.sides[side.0 as usize]
            .conditions
            .as_slice()
            .iter()
            .copied()
            .find(|c| self.state.effects.cells[c.0 as usize].id == status)
    }
    /// SideEnd then cell retirement
    /// Ports `sim/side.ts:456-462`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn remove_side_condition(&mut self, side: SideId, status: EffectId) -> bool {
        let status = self.condition_id(status);
        let Some(c) = self.get_side_condition_data(side, status) else {
            return false;
        };
        let effect = self.condition_ref(status);
        self.mutation_single(
            EventId::SideEnd,
            effect,
            c,
            EventArg::Holder(Holder::side(side)),
            Attribution::NONE,
        );
        self.erase_side_condition(side, status);
        true
    }
    /// Slot conditions retain slotCondition flag and side target
    /// Ports `sim/side.ts:464-494`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn add_slot_condition(
        &mut self,
        slot: SlotId,
        status: EffectId,
        attribution: Attribution,
    ) -> Relay {
        let side = slot.side();
        let a = self.side_condition_attribution(side, attribution);
        let status = self.condition_id(status);
        let effect = self.condition_ref(status);
        if let Some(c) = self.get_slot_condition(slot, status) {
            if self
                .event_hook(effect, EventId::Restart, dex::HookRel::On)
                .is_none()
            {
                return Relay::Bool(false);
            }
            return self.mutation_single(
                EventId::Restart,
                effect,
                c,
                EventArg::Holder(Holder::side(side)),
                a,
            );
        }
        let c = self.mutation_cell(Holder::side(side), Holder::side(side), status);
        self.state.sides[side.0 as usize].slot_conditions[slot.position() as usize].push(c);
        self.state.effects.cells[c.0 as usize].present |= present::SLOT_CONDITION;
        self.condition_source(c, a, true, false);
        self.condition_duration(
            c,
            effect,
            mon_arg(Some(self.state.sides[side.0 as usize].active[0])),
            a,
        );
        let mon = self.state.sides[side.0 as usize].active[slot.position() as usize];
        if !self
            .mutation_single(
                EventId::Start,
                effect,
                c,
                mon_arg((mon != MonId::NONE).then_some(mon)),
                a,
            )
            .truthy()
        {
            self.erase_slot_condition(slot, status);
            return Relay::Bool(false);
        }
        Relay::Bool(true)
    }
    /// Slot-state lookup
    /// Ports `sim/side.ts:496-502`. PRNG: none.
    pub fn get_slot_condition(&self, slot: SlotId, status: EffectId) -> Option<CellId> {
        let status = self.condition_id(status);
        self.state.sides[slot.side().0 as usize].slot_conditions[slot.position() as usize]
            .as_slice()
            .iter()
            .copied()
            .find(|c| self.state.effects.cells[c.0 as usize].id == status)
    }
    /// End with current slot Pokemon before removal
    /// Ports `sim/side.ts:504-510`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn remove_slot_condition(&mut self, slot: SlotId, status: EffectId) -> bool {
        let status = self.condition_id(status);
        let Some(c) = self.get_slot_condition(slot, status) else {
            return false;
        };
        let effect = self.condition_ref(status);
        let mon = self.state.sides[slot.side().0 as usize].active[slot.position() as usize];
        self.mutation_single(
            EventId::End,
            effect,
            c,
            mon_arg((mon != MonId::NONE).then_some(mon)),
            Attribution::NONE,
        );
        self.erase_slot_condition(slot, status);
        true
    }
    /// TryWeather/FieldStart/WeatherChange, old-cell rollback
    /// Ports `sim/field.ts:39-89`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn set_weather(&mut self, status: EffectId, attribution: Attribution) -> Relay {
        let status = self.condition_id(status);
        let effect = self.condition_ref(status);
        let a = self.mutation_attribution(attribution, true, true);
        let prev = self.state.field.weather;
        if self.state.effects.cells[prev.0 as usize].id == status {
            return Relay::Bool(false);
        }
        if let Some(s) = Self::arg_mon(a.source) {
            let r = self.mutation_event(
                EventId::SetWeather,
                mon_arg(Some(s)),
                Attribution {
                    source: mon_arg(Some(s)),
                    effect,
                },
                Relay::Undefined,
            );
            if !r.truthy() {
                if r == Relay::Bool(false) {
                    let id = self.event_effect_id(a.effect);
                    let weather = match a.effect {
                        EffectRef::ActiveMove(h) => {
                            self.scratch.moves[h as usize].unwrap().effects.weather
                        }
                        EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
                            dex::move_data(id).effects.weather
                        }
                        _ => EffectId::NONE,
                    };
                    let old = self.state.effects.cells[prev.0 as usize].id;
                    let old_name = if old == EffectId::NONE {
                        ""
                    } else {
                        dex::effect(old).key
                    };
                    if weather != EffectId::NONE {
                        self.add(LogEntry::new(
                            "-fail",
                            &[LogArg::Mon(s), LogArg::Effect(a.effect)],
                            &[LogTag::Value("from", LogArg::Text(old_name))],
                        ));
                    } else if self.event_effect_type(a.effect) == dex::EffectType::Ability {
                        self.add(LogEntry::new(
                            "-ability",
                            &[LogArg::Mon(s), LogArg::Effect(a.effect)],
                            &[
                                LogTag::Value("from", LogArg::Text(old_name)),
                                LogTag::Bare("fail"),
                            ],
                        ));
                    }
                }
                return Relay::Null;
            }
        }
        let c = self.mutation_cell(Holder::FIELD, Holder::NONE, status);
        self.state.field.weather = c;
        self.condition_source(c, a, true, false);
        if self
            .event_hook(effect, EventId::DurationCallback, dex::HookRel::Direct)
            .is_some()
        {
            assert!(
                Self::arg_mon(a.source).is_some(),
                "setting weather without a source"
            );
        }
        self.condition_duration(c, effect, a.source, a);
        if !self
            .mutation_single(
                EventId::FieldStart,
                effect,
                c,
                EventArg::Holder(Holder::FIELD),
                a,
            )
            .truthy()
        {
            self.state.field.weather = prev;
            self.release_cell(c);
            return Relay::Bool(false);
        }
        self.release_cell(prev);
        self.each_event(EventId::WeatherChange, a.effect, Relay::Undefined);
        Relay::Bool(true)
    }
    /// FieldEnd and state reset
    /// Ports `sim/field.ts:91-99`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn clear_weather(&mut self) -> bool {
        let c = self.state.field.weather;
        let status = self.state.effects.cells[c.0 as usize].id;
        if status == EffectId::NONE {
            return false;
        }
        let effect = self.condition_ref(status);
        self.mutation_single(
            EventId::FieldEnd,
            effect,
            c,
            EventArg::Holder(Holder::FIELD),
            Attribution::NONE,
        );
        self.state.effects.cells[self.state.field.weather.0 as usize].clear();
        self.each_event(EventId::WeatherChange, EffectRef::None, Relay::Undefined);
        true
    }
    /// Weather is absent while weather-suppressing ability is active
    /// Ports `sim/field.ts:101-108`. PRNG: none.
    pub fn field_effective_weather(&mut self) -> EffectId {
        if self.suppressing_weather() || self.state.field.weather == CellId::NONE {
            EffectId::NONE
        } else {
            self.state.effects.cells[self.state.field.weather.0 as usize].id
        }
    }
    /// Living active Air Lock/Cloud Nine check
    /// Ports `sim/field.ts:110-116`. PRNG: none.
    pub fn suppressing_weather(&self) -> bool {
        for side in &self.state.sides {
            for mon in side.active {
                if mon == MonId::NONE {
                    continue;
                }
                let pokemon = &self.state.pokemon[mon.0 as usize];
                if pokemon.flags & crate::state::mon_flags::FAINTED != 0
                    || self.ignoring_ability(mon)
                {
                    continue;
                }
                if pokemon.ability != EffectId::NONE
                    && crate::dex::ABILITIES
                        [(pokemon.ability.0 - crate::dex::ABILITY_START) as usize]
                        .suppress_weather
                    && (pokemon.ability_state == CellId::NONE
                        || self.state.effects.cells[pokemon.ability_state.0 as usize].present
                            & crate::state::present::ENDING
                            == 0)
                {
                    return true;
                }
            }
        }
        false
    }
    /// Compare effective weather
    /// Ports `sim/field.ts:118-128`. PRNG: none.
    pub fn is_weather(&mut self, weather: &[EffectId]) -> bool {
        let effective = self.field_effective_weather();
        weather.contains(&effective)
    }
    /// FieldStart then TerrainChange and per-holder updates
    /// Ports `sim/field.ts:130-157`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn set_terrain(&mut self, status: EffectId, attribution: Attribution) -> bool {
        let status = self.condition_id(status);
        let effect = self.condition_ref(status);
        let a = self.mutation_attribution(attribution, true, true);
        assert!(
            Self::arg_mon(a.source).is_some(),
            "setting terrain without a source"
        );
        let prev = self.state.field.terrain;
        if self.state.effects.cells[prev.0 as usize].id == status {
            return false;
        }
        let c = self.mutation_cell(Holder::FIELD, Holder::NONE, status);
        self.state.field.terrain = c;
        self.condition_source(c, a, true, false);
        self.condition_duration(c, effect, a.source, a);
        if !self
            .mutation_single(
                EventId::FieldStart,
                effect,
                c,
                EventArg::Holder(Holder::FIELD),
                a,
            )
            .truthy()
        {
            self.state.field.terrain = prev;
            self.release_cell(c);
            return false;
        }
        self.release_cell(prev);
        self.each_event(EventId::TerrainChange, a.effect, Relay::Undefined);
        true
    }
    /// FieldEnd then reset
    /// Ports `sim/field.ts:159-167`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn clear_terrain(&mut self) -> bool {
        let c = self.state.field.terrain;
        let status = self.state.effects.cells[c.0 as usize].id;
        if status == EffectId::NONE {
            return false;
        }
        let effect = self.condition_ref(status);
        self.mutation_single(
            EventId::FieldEnd,
            effect,
            c,
            EventArg::Holder(Holder::FIELD),
            Attribution::NONE,
        );
        self.state.effects.cells[self.state.field.terrain.0 as usize].clear();
        self.each_event(EventId::TerrainChange, EffectRef::None, Relay::Undefined);
        true
    }
    /// TryTerrain for a target, field terrain for no target
    /// Ports `sim/field.ts:169-172`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn effective_terrain(&mut self, target: Option<MonId>) -> EffectId {
        let target = target
            .map(|m| EventArg::Holder(Holder::mon(m)))
            .or_else(|| self.mutation_frame().map(|f| f.target))
            .unwrap_or(EventArg::Undefined);
        if self
            .mutation_event(
                EventId::TryTerrain,
                target,
                Attribution::DEFAULT,
                Relay::Undefined,
            )
            .truthy()
        {
            self.state.effects.cells[self.state.field.terrain.0 as usize].id
        } else {
            EffectId::NONE
        }
    }
    /// Compare effective terrain
    /// Ports `sim/field.ts:174-184`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn is_terrain(&mut self, terrain: &[EffectId], target: Option<MonId>) -> bool {
        let current = self.effective_terrain(target);
        terrain.contains(&current)
    }
    /// FieldRestart or duration callback/FieldStart installation
    /// Ports `sim/field.ts:186-215`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn add_pseudo_weather(&mut self, status: EffectId, attribution: Attribution) -> Relay {
        let status = self.condition_id(status);
        let effect = self.condition_ref(status);
        let a = self.mutation_attribution(attribution, true, false);
        if let Some(c) = self.get_pseudo_weather(status) {
            if self
                .event_hook(effect, EventId::FieldRestart, dex::HookRel::On)
                .is_none()
            {
                return Relay::Bool(false);
            }
            return self.mutation_single(
                EventId::FieldRestart,
                effect,
                c,
                EventArg::Holder(Holder::FIELD),
                a,
            );
        }
        let c = self.mutation_cell(Holder::FIELD, Holder::NONE, status);
        self.state.field.pseudo_weather.push(c);
        self.condition_source(c, a, true, false);
        // The initializer includes these keys even for an undefined source.
        self.state.effects.cells[c.0 as usize].present |= present::SOURCE | present::SOURCE_SLOT;
        if self
            .event_hook(effect, EventId::DurationCallback, dex::HookRel::Direct)
            .is_some()
        {
            assert!(
                Self::arg_mon(a.source).is_some(),
                "setting fieldcond without a source"
            );
        }
        self.condition_duration(c, effect, a.source, a);
        if !self
            .mutation_single(
                EventId::FieldStart,
                effect,
                c,
                EventArg::Holder(Holder::FIELD),
                a,
            )
            .truthy()
        {
            self.erase_pseudo_weather(status);
            return Relay::Bool(false);
        }
        self.mutation_event(
            EventId::PseudoWeatherChange,
            a.source,
            Attribution { effect, ..a },
            Relay::Undefined,
        );
        Relay::Bool(true)
    }
    /// Ordered field-list lookup
    /// Ports `sim/field.ts:217-221`. PRNG: none.
    pub fn get_pseudo_weather(&self, status: EffectId) -> Option<CellId> {
        let status = self.condition_id(status);
        self.state
            .field
            .pseudo_weather
            .as_slice()
            .iter()
            .copied()
            .find(|c| self.state.effects.cells[c.0 as usize].id == status)
    }
    /// FieldEnd then removal
    /// Ports `sim/field.ts:223-229`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn remove_pseudo_weather(&mut self, status: EffectId) -> bool {
        let status = self.condition_id(status);
        let Some(c) = self.get_pseudo_weather(status) else {
            return false;
        };
        let effect = self.condition_ref(status);
        self.mutation_single(
            EventId::FieldEnd,
            effect,
            c,
            EventArg::Holder(Holder::FIELD),
            Attribution::NONE,
        );
        self.erase_pseudo_weather(status);
        true
    }
}

impl<L: LogSink> Battle<L> {
    fn side_condition_attribution(&self, side: SideId, a: Attribution) -> Attribution {
        let mut a = self.mutation_attribution(a, true, false);
        if let EventArg::Holder(h) = a.source {
            if h.0 == 12 || h.0 == 13 {
                a.source = mon_arg(Some(self.state.sides[(h.0 - 12) as usize].active[0]));
            }
        }
        assert!(
            Self::arg_mon(a.source).is_some(),
            "setting sidecond without a source"
        );
        a
    }
    fn erase_side_condition(&mut self, side: SideId, status: EffectId) {
        let list = &self.state.sides[side.0 as usize].conditions;
        if let Some(i) = list
            .as_slice()
            .iter()
            .position(|c| self.state.effects.cells[c.0 as usize].id == status)
        {
            let c = self.state.sides[side.0 as usize].conditions.remove(i);
            self.release_cell(c);
        }
    }
    fn erase_slot_condition(&mut self, slot: SlotId, status: EffectId) {
        let list =
            &self.state.sides[slot.side().0 as usize].slot_conditions[slot.position() as usize];
        if let Some(i) = list
            .as_slice()
            .iter()
            .position(|c| self.state.effects.cells[c.0 as usize].id == status)
        {
            let c = self.state.sides[slot.side().0 as usize].slot_conditions
                [slot.position() as usize]
                .remove(i);
            self.release_cell(c);
        }
    }
    fn erase_pseudo_weather(&mut self, status: EffectId) {
        if let Some(i) = self
            .state
            .field
            .pseudo_weather
            .as_slice()
            .iter()
            .position(|c| self.state.effects.cells[c.0 as usize].id == status)
        {
            let c = self.state.field.pseudo_weather.remove(i);
            self.release_cell(c);
        }
    }
}
