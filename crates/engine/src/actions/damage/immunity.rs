//! Type, effectiveness and immunity ports from pinned Showdown.
#![allow(unused_variables, unused_imports)]
use crate::{
    Battle,
    actions::*,
    dex::{self, ImmunityId},
    event::{EffectRef, EventArg, Relay, RunEventOptions},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{
        CellId,
        scratch::{HitData, OrderedBoosts},
    },
};
impl<L: LogSink> Battle<L> {
    /// Run per-type Effectiveness hooks then sum modifiers
    /// Ports `sim/pokemon.ts:2208-2234`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn run_effectiveness(&mut self, target: MonId, move_handle: MoveHandle) -> i8 {
        let m = *self.active_move(move_handle);
        let e = EffectRef::ActiveMove(move_handle.0);
        let mut total = 0i8;
        if self.state.pokemon[target.0 as usize].terastallized != TypeId::NONE
            && m.move_type == dex::TYPE_STELLAR
        {
            total = 1;
        } else {
            let types = self.get_types(target, false, false);
            for &ty in &types.values[..types.len as usize] {
                let n = match dex::TYPE_CHART[(ty.0 - 1) as usize][(m.move_type.0 - 1) as usize] {
                    1 => 1.,
                    2 => -1.,
                    _ => 0.,
                };
                let r = self.single_event(
                    EventId::Effectiveness,
                    e,
                    None,
                    EventArg::Holder(Holder::mon(target)),
                    EventArg::Type(ty),
                    e,
                    Relay::Number(n),
                    None,
                );
                let r = self.run_event(
                    EventId::Effectiveness,
                    EventArg::Holder(Holder::mon(target)),
                    EventArg::Type(ty),
                    e,
                    r,
                    RunEventOptions::default(),
                );
                total += super::super::mutators::common::number(r) as i8;
            }
        }
        if self.state.pokemon[target.0 as usize].species == dex::SPECIES_TERAPAGOSTERASTAL
            && self.query_has_ability(target, "terashell")
            && !self.suppressing_ability(Some(target))
        {
            let cell = self.state.pokemon[target.0 as usize].ability_state;
            const RESISTED: u32 = 1 << crate::state::present::CUSTOM_START;
            if m.hit == 1 {
                self.state.effects.cells[cell.0 as usize].present &= !RESISTED;
            }
            if self.state.effects.cells[cell.0 as usize].present & RESISTED != 0 {
                return -1;
            }
            if m.category == dex::Category::Status
                || m.id == dex::MOVE_STRUGGLE
                || !self.run_immunity(
                    target,
                    ImmunitySource::Move(move_handle),
                    ImmunityMessage::Silent,
                )
                || total < 0
                || self.state.pokemon[target.0 as usize].hp
                    < self.state.pokemon[target.0 as usize].max_hp
            {
                return total;
            }
            self.add(LogEntry::new(
                "-activate",
                &[LogArg::Mon(target), LogArg::Text("ability: Tera Shell")],
                &[],
            ));
            self.state.effects.cells[cell.0 as usize].present |= RESISTED;
            return -1;
        }
        total
    }
    /// Type immunity including groundedness and ignore-immunity events
    /// Ports `sim/pokemon.ts:2236-2267`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn run_immunity(
        &mut self,
        target: MonId,
        source: ImmunitySource,
        message: ImmunityMessage,
    ) -> bool {
        let ty = match source {
            ImmunitySource::Type(t) => t,
            ImmunitySource::Move(h) => {
                let m = self.active_move(h);
                if m.runtime_flags & crate::state::scratch::move_runtime::IGNORE_IMMUNITY != 0
                    || m.ignore_immunity_types & (1 << m.move_type.0) != 0
                {
                    return true;
                }
                m.move_type
            }
        };
        if ty == TypeId::NONE || ty == dex::TYPE_UNKNOWN {
            return true;
        }
        let negate = !self
            .run_event(
                EventId::NegateImmunity,
                EventArg::Holder(Holder::mon(target)),
                EventArg::Type(ty),
                EffectRef::None,
                Relay::Undefined,
                RunEventOptions::default(),
            )
            .truthy();
        let immune = if ty == dex::TYPE_GROUND {
            self.is_grounded(target, negate)
        } else {
            let types = self.get_types(target, false, false);
            Relay::Bool(
                negate
                    || types.values[..types.len as usize]
                        .iter()
                        .all(|t| dex::TYPE_CHART[(t.0 - 1) as usize][(ty.0 - 1) as usize] != 3),
            )
        };
        if immune.truthy() {
            return true;
        }
        if !matches!(message, ImmunityMessage::Silent) {
            let tags: &[LogTag<'_>] =
                if immune == Relay::Null && self.query_has_ability(target, "levitate") {
                    &[LogTag::Value("from", LogArg::Text("ability: Levitate"))]
                } else {
                    &[]
                };
            self.add(LogEntry::new("-immune", &[LogArg::Mon(target)], tags));
        }
        false
    }
    /// Status/powder/sandstorm immunity has a separate name space
    /// Ports `sim/pokemon.ts:2269-2294`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn run_status_immunity(
        &mut self,
        target: MonId,
        immunity: ImmunityId,
        message: ImmunityMessage,
    ) -> bool {
        if self.state.pokemon[target.0 as usize].flags & crate::state::mon_flags::FAINTED != 0 {
            return false;
        }
        let types = self.get_types(target, false, false);
        if types.values[..types.len as usize]
            .iter()
            .any(|t| dex::IMMUNITY_CHART[(t.0 - 1) as usize][immunity as usize] == 3)
        {
            if !matches!(message, ImmunityMessage::Silent) {
                self.add(LogEntry::new("-immune", &[LogArg::Mon(target)], &[]));
            }
            return false;
        }
        let r = self.run_event(
            EventId::Immunity,
            EventArg::Holder(Holder::mon(target)),
            EventArg::Null,
            EffectRef::None,
            Relay::StatusImmunity(immunity),
            RunEventOptions::default(),
        );
        if !r.truthy() {
            if !matches!(message, ImmunityMessage::Silent) && r != Relay::Null {
                self.add(LogEntry::new("-immune", &[LogArg::Mon(target)], &[]));
            }
            return false;
        }
        true
    }
    /// Includes added type, Tera and Type events
    /// Ports `sim/pokemon.ts:2138-2146`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn get_types(&mut self, pokemon: MonId, exclude_added: bool, pre_tera: bool) -> Types {
        let mon = &self.state.pokemon[pokemon.0 as usize];
        let stellar = dex::TYPE_STELLAR;
        if !pre_tera && mon.terastallized != TypeId::NONE && mon.terastallized != stellar {
            return Types {
                values: [mon.terastallized, TypeId::NONE, TypeId::NONE],
                len: 1,
            };
        }
        let mut base = Types {
            values: [TypeId::NONE; 3],
            len: 0,
        };
        for &ty in &mon.types {
            if ty != TypeId::NONE {
                base.values[base.len as usize] = ty;
                base.len += 1;
            }
        }
        let original = self.stash_types(base);
        let result = self.run_event(
            EventId::Type,
            EventArg::Holder(Holder::mon(pokemon)),
            EventArg::Null,
            EffectRef::None,
            Relay::Types(original),
            RunEventOptions::default(),
        );
        let Relay::Types(handle) = result else {
            panic!("Type event returned a non-array relay");
        };
        if self.scratch_types(handle).len == 0 {
            let types = self.scratch_types(handle);
            types.values[0] = TypeId(1); // Gen 9 empty-array fallback is Normal.
            types.len = 1;
        }
        let mut types = *self.scratch_types(handle);
        // The input relay aliases Pokemon.types in JS. Preserve mutation of that
        // original array, even when a handler returns a different array.
        let original_types = *self.scratch_types(original);
        assert!(
            original_types.len <= 2,
            "Type event expanded persistent types past snapshot capacity"
        );
        self.state.pokemon[pokemon.0 as usize].types = [
            if original_types.len > 0 {
                original_types.values[0]
            } else {
                TypeId::NONE
            },
            if original_types.len > 1 {
                original_types.values[1]
            } else {
                TypeId::NONE
            },
        ];
        self.release_relay(result);
        if handle != original {
            self.release_relay(Relay::Types(original));
        }
        let added = self.state.pokemon[pokemon.0 as usize].added_type;
        if !exclude_added && added != TypeId::NONE {
            assert!(
                (types.len as usize) < types.values.len(),
                "Type array capacity exceeded"
            );
            types.values[types.len as usize] = added;
            types.len += 1;
        }
        types
    }
    /// Null distinguishes Levitate from explicit ungrounded false
    /// Ports `sim/pokemon.ts:2148-2160`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn is_grounded(&mut self, pokemon: MonId, negate_immunity: bool) -> Relay {
        let gravity = self
            .state
            .field
            .pseudo_weather
            .as_slice()
            .iter()
            .any(|c| dex::effect(self.state.effects.cells[c.0 as usize].id).key == "gravity");
        if gravity
            || self.query_has_volatile(pokemon, "ingrain")
            || self.query_has_volatile(pokemon, "smackdown")
        {
            return Relay::Bool(true);
        }
        // Pokemon.isGrounded reads the effective item only once, before types.
        let item = if self.ignoring_item(pokemon) {
            EffectId::NONE
        } else {
            self.state.pokemon[pokemon.0 as usize].item
        };
        let item_key = if item == EffectId::NONE {
            ""
        } else {
            dex::effect(item).key
        };
        if item_key == "ironball" {
            return Relay::Bool(true);
        }
        if !negate_immunity
            && self.has_type(pokemon, &[TypeId(3)])
            && !(self.has_type(pokemon, &[TypeId(20)]) && self.query_has_volatile(pokemon, "roost"))
        {
            return Relay::Bool(false);
        }
        if (self.query_has_ability(pokemon, "levitate")
            || self.query_has_ability(pokemon, "eelevate"))
            && !self.suppressing_ability(Some(pokemon))
        {
            return Relay::Null;
        }
        if self.query_has_volatile(pokemon, "magnetrise")
            || self.query_has_volatile(pokemon, "telekinesis")
        {
            return Relay::Bool(false);
        }
        Relay::Bool(item_key != "airballoon")
    }
    /// Semi-invulnerability conditions and the Sky Drop source relation
    /// Ports `sim/pokemon.ts:2162-2188`. PRNG: none.
    pub fn is_semi_invulnerable(&self, pokemon: MonId) -> bool {
        if [
            "fly",
            "bounce",
            "dive",
            "dig",
            "phantomforce",
            "shadowforce",
            "skydrop",
        ]
        .iter()
        .any(|key| self.query_has_volatile(pokemon, key))
        {
            return true;
        }
        // isSkyDropped also follows the opposing holder's common source field.
        let foe = (pokemon.side().0 ^ 1) as usize;
        for &active in &self.state.sides[foe].active {
            if active == MonId::NONE {
                continue;
            }
            let list = &self.state.pokemon[active.0 as usize].volatiles;
            if list.as_slice().iter().any(|c| {
                let cell = &self.state.effects.cells[c.0 as usize];
                dex::effect(cell.id).key == "skydrop" && cell.source == pokemon
            }) {
                return true;
            }
        }
        false
    }
    /// Field suppression plus Utility Umbrella; Mega Sol uses current effect attribution.
    /// Ports `sim/pokemon.ts:2190-2205`. PRNG: none.
    pub fn effective_weather(&mut self, pokemon: MonId) -> EffectId {
        let weather = self.field_effective_weather();
        let source_effect = self.scratch.current_effect;
        let source_id = self.event_effect_id(source_effect);
        let mega_sol_effect =
            source_id != EffectId::NONE && dex::effect(source_id).key == "megasol";
        if self.scratch.active_pokemon != MonId::NONE
            && self.query_has_ability(self.scratch.active_pokemon, "megasol")
            && source_effect != EffectRef::None
            && (mega_sol_effect
                || matches!(
                    self.event_effect_type(source_effect),
                    dex::EffectType::Move | dex::EffectType::Weather
                ))
        {
            // The existing API has the source default message=false; Mega Sol
            // itself is excluded from the generated executable ability closure.
            return dex::CONDITION_SUNNYDAY;
        }
        if weather != EffectId::NONE
            && matches!(
                dex::effect(weather).key,
                "sunnyday" | "raindance" | "desolateland" | "primordialsea"
            )
            && self.query_has_item(pokemon, "utilityumbrella")
        {
            return EffectId::NONE;
        }
        weather
    }
}
impl<L: LogSink> Battle<L> {
    /// Ports sim/pokemon.ts:1567-1579. PRNG: Type handlers/events; preserve repeated queries.
    pub fn has_type(&mut self, pokemon: MonId, types: &[TypeId]) -> bool {
        let current = self.get_types(pokemon, false, false);
        types
            .iter()
            .any(|ty| current.values[..current.len as usize].contains(ty))
    }
}
