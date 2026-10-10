//! Pokemon read/contact helpers ported from pinned Showdown; other mutators remain explicit stubs.
#![allow(unused_variables, unused_imports)]
use super::common::{mon_arg, number};
use crate::{
    Battle,
    actions::*,
    dex::{self, ImmunityId},
    event::{EffectRef, EventArg, Relay, RunEventOptions},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{
        CellId, mon_flags, present,
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
        let r = self.mutation_event(
            EventId::ModifySpecies,
            mon_arg(Some(pokemon)),
            Attribution {
                source: EventArg::Null,
                effect: source_effect,
            },
            Relay::Species(species),
        );
        if !r.truthy() {
            return Relay::Null;
        }
        let Relay::Species(actual) = r else {
            panic!("ModifySpecies returned non-species");
        };
        let data = dex::species(actual);
        let raw = dex::species(species);
        let stats = crate::battle::compute_stats(
            data,
            &self.teams.sides[pokemon.side().0 as usize].sets[(pokemon.0 % 6) as usize],
        )
        .expect("species stats overflow");
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        p.species = actual;
        p.types = data.types;
        p.apparent_types = raw.types;
        p.added_type = TypeId::NONE;
        p.flags |= mon_flags::KNOWN_TYPE;
        p.weighthg = data.weighthg;
        if p.max_hp == 0 {
            p.max_hp = stats[0];
            p.hp = stats[0];
        }
        if !is_transform {
            p.base_stored_stats = stats[1..].try_into().unwrap();
        }
        p.stored_stats = stats[1..].try_into().unwrap();
        p.speed = p.stored_stats[4];
        r
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
        let r = self.set_species(pokemon, species, source_effect, false);
        let Relay::Species(actual) = r else {
            return false;
        };
        let data = dex::species(actual);
        let illusion = self.state.pokemon[pokemon.0 as usize].illusion;
        if options.permanent {
            self.state.pokemon[pokemon.0 as usize].base_species = species;
            self.add(LogEntry::new(
                "detailschange",
                &[
                    LogArg::Mon(pokemon),
                    LogArg::Details(if illusion == MonId::NONE {
                        pokemon
                    } else {
                        illusion
                    }),
                ],
                &[],
            ));
            self.update_max_hp(pokemon);
            if source_effect == EffectRef::None {
                self.state.pokemon[pokemon.0 as usize].flags |= mon_flags::FORME_REGRESSION;
            } else if self.event_effect_type(source_effect) == dex::EffectType::Item {
                let p = &mut self.state.pokemon[pokemon.0 as usize];
                p.flags |= mon_flags::TERA_BLOCKED | mon_flags::FORME_REGRESSION;
                p.move_this_turn_result = crate::state::ResultFlag::True;
                let apparent = if illusion != MonId::NONE {
                    self.state.pokemon[illusion.0 as usize].species
                } else {
                    data.base_species
                };
                let required = data
                    .required_items
                    .first()
                    .copied()
                    .unwrap_or(EffectId::NONE);
                let primal = dex::effect(self.event_effect_id(source_effect))
                    .data
                    .get(dex::FIELD_ISPRIMALORB)
                    == Some(dex::DataValue::Bool(true));
                self.add(LogEntry::new(
                    if primal { "-primal" } else { "-mega" },
                    &[
                        LogArg::Mon(pokemon),
                        LogArg::Species(apparent),
                        if required == EffectId::NONE {
                            LogArg::Empty
                        } else {
                            LogArg::Effect(EffectRef::Dex(required))
                        },
                    ],
                    &[],
                ));
            } else if self.event_effect_type(source_effect) == dex::EffectType::Status {
                self.log_forme(pokemon, actual, options.message, None);
            }
        } else {
            let ability = (self.event_effect_type(source_effect) == dex::EffectType::Ability)
                .then_some(source_effect);
            let shown = if ability.is_none() && illusion != MonId::NONE {
                self.state.pokemon[illusion.0 as usize].species
            } else {
                actual
            };
            self.log_forme(pokemon, shown, options.message, ability);
        }
        let source_id = self.event_effect_id(source_effect);
        if options.permanent && ![dex::ABILITY_DISGUISE, dex::ABILITY_ICEFACE].contains(&source_id)
        {
            if illusion != MonId::NONE && source_effect != EffectRef::None {
                self.state.pokemon[pokemon.0 as usize].ability = EffectId::NONE;
            }
            let ability = data.abilities[0].id;
            let old = self.state.pokemon[pokemon.0 as usize].ability;
            let cant = old != EffectId::NONE
                && dex::ABILITIES[(old.0 - dex::ABILITY_START) as usize].flags
                    & dex::FLAG_CANTSUPPRESS
                    != 0;
            if source_effect != EffectRef::None || !cant {
                self.set_ability(pokemon, ability, Attribution::NONE, true, false);
            }
            self.state.pokemon[pokemon.0 as usize].base_ability = ability;
        }
        self.tera_apparent_type(pokemon);
        true
    }
    /// Virtual move PP/types/boosts/ability; preserve real base slot ownership
    /// Ports `sim/pokemon.ts:1270-1385`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn transform_into(&mut self, pokemon: MonId, target: MonId, effect: EffectRef) -> bool {
        let p = self.state.pokemon[pokemon.0 as usize];
        let t = self.state.pokemon[target.0 as usize];
        let species = dex::species(t.species);
        if t.flags & mon_flags::FAINTED != 0
            || p.illusion != MonId::NONE
            || t.illusion != MonId::NONE
            || self
                .get_volatile(target, dex::CONDITION_SUBSTITUTE)
                .is_some()
            || t.flags & mon_flags::TRANSFORMED != 0
            || p.flags & mon_flags::TRANSFORMED != 0
            || species.name == "Eternatus-Eternamax"
            || (matches!(species.base_species_name, "Ogerpon" | "Terapagos")
                && (p.terastallized != TypeId::NONE || t.terastallized != TypeId::NONE))
            || p.terastallized == dex::type_id("Stellar").unwrap()
        {
            return false;
        }
        if !self.set_species(pokemon, t.species, effect, true).truthy() {
            return false;
        }
        let mut types = self.get_types(target, true, true);
        if let Some(c) = self.get_volatile(target, dex::CONDITION_ROOST) {
            // Roost typeWas is a packed pre-Roost Types payload (word 0).
            let word = self.state.effects.cells[c.0 as usize].payload.words[0];
            types.values = [TypeId(word as u8), TypeId((word >> 8) as u8), TypeId::NONE];
            types.len = if types.values[1] == TypeId::NONE {
                1
            } else {
                2
            };
        }
        self.set_type(pokemon, types, true);
        let mut slots = [crate::state::MoveSlot::default(); 4];
        let original = t.move_slots();
        for (i, s) in original.iter().enumerate() {
            let pp = dex::move_data(s.id).pp.min(5);
            slots[i] = crate::state::MoveSlot {
                id: s.id,
                pp,
                max_pp: pp,
                target: s.target,
                flags: SLOT_VIRTUAL,
                ..Default::default()
            };
        }
        let receiver = &mut self.state.pokemon[pokemon.0 as usize];
        receiver.flags |= mon_flags::TRANSFORMED;
        receiver.weighthg = t.weighthg;
        receiver.added_type = t.added_type;
        if pokemon.side() == target.side() && t.flags & mon_flags::KNOWN_TYPE != 0 {
            receiver.flags |= mon_flags::KNOWN_TYPE;
        } else {
            receiver.flags &= !mon_flags::KNOWN_TYPE;
        }
        receiver.apparent_types = t.apparent_types;
        receiver.stored_stats = t.stored_stats;
        receiver.times_attacked = t.times_attacked;
        receiver.boosts = t.boosts;
        receiver.virtual_move_slots = slots;
        receiver.virtual_move_count = original.len() as u8;
        for key in ["dragoncheer", "focusenergy", "gmaxchistrike", "laserfocus"] {
            if let Some(id) = dex::lookup(EffectKind::Condition, key) {
                self.remove_volatile(pokemon, id);
            }
        }
        for key in ["dragoncheer", "focusenergy", "gmaxchistrike", "laserfocus"] {
            if let Some(id) = dex::lookup(EffectKind::Condition, key) {
                if let Some(c) = self.get_volatile(target, id) {
                    let payload = self.state.effects.cells[c.0 as usize].payload;
                    self.add_volatile(pokemon, id, Attribution::NONE, None);
                    if key == "dragoncheer" || key == "gmaxchistrike" {
                        let d = self
                            .get_volatile(pokemon, id)
                            .expect("critical volatile rejected");
                        self.state.effects.cells[d.0 as usize].payload = payload;
                    }
                }
            }
        }
        let tags: &[LogTag<'_>] = if effect != EffectRef::None {
            &[LogTag::From(effect)]
        } else {
            &[]
        };
        self.add(LogEntry::new(
            "-transform",
            &[LogArg::Mon(pokemon), LogArg::Mon(target)],
            tags,
        ));
        self.tera_apparent_type(pokemon);
        self.set_ability(
            pokemon,
            t.ability,
            Attribution::from_move(pokemon, EffectRef::None),
            true,
            true,
        );
        if matches!(
            dex::species(self.state.pokemon[pokemon.0 as usize].species).base_species_name,
            "Ogerpon" | "Terapagos"
        ) {
            self.state.pokemon[pokemon.0 as usize].flags |= mon_flags::TERA_BLOCKED;
        }
        true
    }
    /// Baton Pass/Shed Tail filtered boost+volatile transfer and relinking
    /// Ports `sim/pokemon.ts:1246-1268`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn copy_volatile_from(
        &mut self,
        pokemon: MonId,
        source: MonId,
        switch_effect: EffectId,
    ) -> () {
        self.clear_volatile(pokemon, true);
        let shedtail =
            switch_effect != EffectId::NONE && dex::effect(switch_effect).key == "shedtail";
        if !shedtail {
            self.state.pokemon[pokemon.0 as usize].boosts =
                self.state.pokemon[source.0 as usize].boosts;
        }
        let cells = self.state.pokemon[source.0 as usize].volatiles;
        for &old_cell in cells.as_slice() {
            let old = self.state.effects.cells[old_cell.0 as usize];
            if (shedtail && old.id != dex::CONDITION_SUBSTITUTE)
                || dex::effect(old.id).data.get(dex::FIELD_NOCOPY)
                    == Some(dex::DataValue::Bool(true))
            {
                continue;
            }
            let c = self.mutation_cell(Holder::mon(pokemon), Holder::mon(pokemon), old.id);
            let fresh = self.state.effects.cells[c.0 as usize];
            let mut copied = old;
            copied.generation = fresh.generation;
            copied.owner = fresh.owner;
            copied.target = fresh.target;
            copied.effect_order = fresh.effect_order;
            copied.present |= present::TARGET;
            // Preserve allocation bookkeeping by copying only public semantic fields.
            let dest = &mut self.state.effects.cells[c.0 as usize];
            dest.id = copied.id;
            dest.present = copied.present;
            dest.payload = copied.payload;
            dest.source_effect = copied.source_effect;
            dest.source = copied.source;
            dest.source_slot = copied.source_slot;
            dest.duration = copied.duration;
            dest.linked_status = copied.linked_status;
            dest.linked_mons = copied.linked_mons;
            self.state.pokemon[pokemon.0 as usize].volatiles.push(c);
            if old.linked_status != EffectId::NONE {
                self.state.effects.cells[old_cell.0 as usize].linked_status = EffectId::NONE;
                self.state.effects.cells[old_cell.0 as usize].linked_mons = 0;
                self.state.effects.cells[old_cell.0 as usize].present &= !present::LINKED_STATUS;
                for i in 0..12 {
                    if old.linked_mons & (1 << i) != 0 {
                        let other = self
                            .get_volatile(MonId(i), old.linked_status)
                            .expect("broken volatile link");
                        let c = &mut self.state.effects.cells[other.0 as usize];
                        c.linked_mons = (c.linked_mons & !(1 << source.0)) | (1 << pokemon.0);
                    }
                }
            }
        }
        self.clear_volatile(source, true);
        let cells = self.state.pokemon[pokemon.0 as usize].volatiles;
        for &c in cells.as_slice() {
            let effect = self.condition_ref(self.state.effects.cells[c.0 as usize].id);
            self.mutation_single(
                EventId::Copy,
                effect,
                c,
                mon_arg(Some(pokemon)),
                Attribution::DEFAULT,
            );
        }
    }
    /// Reset Transform, move slots, boosts and links; preserve base data
    /// Ports `sim/pokemon.ts:1508-1567`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn clear_volatile(&mut self, pokemon: MonId, include_switch_flags: bool) -> () {
        self.clear_boosts(pokemon);
        {
            let p = &mut self.state.pokemon[pokemon.0 as usize];
            p.virtual_move_count = 0;
            p.virtual_move_slots = [crate::state::MoveSlot::default(); 4];
            p.flags &= !(mon_flags::TRANSFORMED | mon_flags::TERA_BLOCKED);
            p.ability = p.base_ability;
        }
        let cells = self.state.pokemon[pokemon.0 as usize].volatiles;
        for &c in cells.as_slice() {
            let old = self.state.effects.cells[c.0 as usize];
            if old.linked_status != EffectId::NONE {
                self.unlink_volatile(pokemon, old.linked_status, old.linked_mons);
            }
        }
        let cells = self.state.pokemon[pokemon.0 as usize].volatiles;
        self.state.pokemon[pokemon.0 as usize].volatiles = Default::default();
        for &c in cells.as_slice() {
            self.release_cell(c);
        }
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        if include_switch_flags {
            p.switch_flag = EffectId::NONE;
            p.flags &= !(mon_flags::FORCE_SWITCH | mon_flags::SWITCH_REQUESTED);
        }
        p.last_move = EffectId::NONE;
        p.move_this_turn_result = crate::state::ResultFlag::Undefined;
        p.move_last_turn_result = crate::state::ResultFlag::Undefined;
        p.attacks = [crate::state::AttackRecord::default(); 12];
        p.flags |= mon_flags::NEWLY_SWITCHED;
        p.flags &= !mon_flags::BEING_CALLED_BACK;
        // started is dropped in this format; active-turn logic lives in lifecycle.
        let base = p.base_species;
        self.set_species(pokemon, base, self.scratch.current_effect, false);
    }
    /// Tera/Multitype/RKS restrictions and apparent type state
    /// Ports `sim/pokemon.ts:2109-2130`. PRNG: none.
    pub fn set_type(&mut self, pokemon: MonId, types: Types, enforce: bool) -> bool {
        assert!(types.len > 0 && types.len <= 2, "persistent type capacity");
        let p = &self.state.pokemon[pokemon.0 as usize];
        if !enforce {
            if types.values[..types.len as usize].contains(&dex::type_id("Stellar").unwrap())
                || p.terastallized != TypeId::NONE
            {
                return false;
            }
            if matches!(
                dex::species(p.species).base_species_name,
                "Arceus" | "Silvally"
            ) {
                return false;
            }
        }
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        p.types = [
            types.values[0],
            if types.len == 2 {
                types.values[1]
            } else {
                TypeId::NONE
            },
        ];
        p.apparent_types = p.types;
        p.added_type = TypeId::NONE;
        p.flags |= mon_flags::KNOWN_TYPE;
        true
    }
    /// Added type and knownType flag
    /// Ports `sim/pokemon.ts:2132-2136`. PRNG: none.
    pub fn add_type(&mut self, pokemon: MonId, move_type: TypeId) -> bool {
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        if p.terastallized != TypeId::NONE {
            return false;
        }
        p.added_type = move_type;
        true
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
        let p = &self.state.pokemon[source.0 as usize];
        let t = &self.state.pokemon[target.0 as usize];
        if p.flags & mon_flags::FAINTED != 0 || t.flags & mon_flags::FAINTED != 0 {
            return false;
        }
        let source_ability = p.ability;
        let target_ability = t.ability;
        let fails = |id: EffectId| {
            id != EffectId::NONE
                && dex::ABILITIES[(id.0 - dex::ABILITY_START) as usize].flags
                    & dex::FLAG_FAILSKILLSWAP
                    != 0
        };
        if fails(source_ability) || fails(target_ability) {
            return false;
        }
        let effect = self.condition_ref(dex::CONDITION_SKILLSWAP);
        let a = Attribution::from_move(source, effect);
        if !self
            .mutation_event(
                EventId::SetAbility,
                mon_arg(Some(target)),
                a,
                Relay::Effect(source_ability),
            )
            .truthy()
            || !self
                .mutation_event(
                    EventId::SetAbility,
                    mon_arg(Some(source)),
                    a,
                    Relay::Effect(target_ability),
                )
                .truthy()
        {
            return false;
        }
        let allies = source.side() == target.side();
        let args = [
            LogArg::Mon(source),
            LogArg::Text("Skill Swap"),
            if allies || target_ability == EffectId::NONE {
                LogArg::Empty
            } else {
                LogArg::Effect(EffectRef::Dex(target_ability))
            },
            if allies || source_ability == EffectId::NONE {
                LogArg::Empty
            } else {
                LogArg::Effect(EffectRef::Dex(source_ability))
            },
        ];
        self.add(LogEntry::new("-activate", &args, &[LogTag::Of(target)]));
        if allies {
            self.hint(LogArg::Text("Skill Swap does not announce the abilities of the Pokémon when used between allies."), false, None);
        }
        let sc = self.state.pokemon[source.0 as usize].ability_state;
        let tc = self.state.pokemon[target.0 as usize].ability_state;
        self.mutation_single(
            EventId::End,
            EffectRef::Dex(source_ability),
            sc,
            mon_arg(Some(source)),
            Attribution::DEFAULT,
        );
        self.mutation_single(
            EventId::End,
            EffectRef::Dex(target_ability),
            tc,
            mon_arg(Some(target)),
            Attribution::DEFAULT,
        );
        self.state.pokemon[source.0 as usize].ability = target_ability;
        self.state.pokemon[target.0 as usize].ability = source_ability;
        let new_sc = self.mutation_cell(Holder::mon(source), Holder::mon(source), target_ability);
        let new_tc = self.mutation_cell(Holder::mon(target), Holder::mon(target), source_ability);
        self.state.pokemon[source.0 as usize].ability_state = new_sc;
        self.state.pokemon[target.0 as usize].ability_state = new_tc;
        self.release_cell(sc);
        self.release_cell(tc);
        self.mutation_single(
            EventId::Start,
            EffectRef::Dex(source_ability),
            new_tc,
            mon_arg(Some(target)),
            Attribution::DEFAULT,
        );
        self.mutation_single(
            EventId::Start,
            EffectRef::Dex(target_ability),
            new_sc,
            mon_arg(Some(source)),
            Attribution::DEFAULT,
        );
        true
    }
}
impl<L: LogSink> Battle<L> {
    /// Ports sim/pokemon.ts:1498-1506. PRNG: none directly; stat helpers may dispatch.
    pub fn update_max_hp(&mut self, pokemon: MonId) -> () {
        let p = &self.state.pokemon[pokemon.0 as usize];
        let new = crate::battle::compute_stats(
            dex::species(p.species),
            &self.teams.sides[pokemon.side().0 as usize].sets[(pokemon.0 % 6) as usize],
        )
        .unwrap()[0];
        if new == p.max_hp {
            return;
        }
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        p.hp = if p.hp == 0 {
            0
        } else {
            (new as i32 - (p.max_hp as i32 - p.hp as i32)).max(1) as u16
        };
        p.max_hp = new;
        if p.hp != 0 {
            self.add(LogEntry::split(
                "-heal",
                &[LogArg::Mon(pokemon), LogArg::Health(pokemon)],
                &[LogTag::Bare("silent")],
                pokemon.side(),
                false,
            ));
        }
    }
    /// Ports sim/pokemon.ts:1625-1637. PRNG: none. Preserve hidden versus true disable.
    pub fn disable_move(
        &mut self,
        pokemon: MonId,
        move_id: EffectId,
        hidden: bool,
        source_effect: EffectRef,
    ) -> () {
        let effect = if source_effect == EffectRef::None && self.mutation_frame().is_some() {
            self.scratch.current_effect
        } else {
            source_effect
        };
        let source_id = self.event_effect_id(effect);
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        let slots = if p.flags & mon_flags::TRANSFORMED != 0 {
            &mut p.virtual_move_slots[..p.virtual_move_count as usize]
        } else {
            &mut p.base_move_slots[..p.move_count as usize]
        };
        for s in slots {
            if s.id == move_id && s.flags & SLOT_DISABLED == 0 {
                s.flags &= !(SLOT_DISABLED | SLOT_HIDDEN);
                s.flags |= if hidden { SLOT_HIDDEN } else { SLOT_DISABLED };
                s.disabled_source = if source_id == EffectId::NONE {
                    move_id
                } else {
                    source_id
                };
            }
        }
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
        EffectToken::from_ref(match effect {
            EffectRef::ActiveMove(i) => EffectRef::Dex(
                self.scratch.moves[i as usize]
                    .expect("released move at faint")
                    .id,
            ),
            other => other,
        })
    }
}

#[cfg(test)]
#[path = "queries/tests.rs"]
mod query_tests;

/// MoveSlot flags shared with choices/lifecycle: true disable, hidden disable, used.
pub const SLOT_DISABLED: u8 = 1;
pub const SLOT_HIDDEN: u8 = 2;
pub const SLOT_USED: u8 = 4;

pub const SLOT_VIRTUAL: u8 = 8;
impl<L: LogSink> Battle<L> {
    fn log_forme(
        &mut self,
        mon: MonId,
        species: EffectId,
        message: Option<&'static str>,
        ability: Option<EffectRef>,
    ) {
        let args = [
            LogArg::Mon(mon),
            LogArg::Species(species),
            message.map_or(LogArg::Empty, LogArg::Text),
        ];
        let tags: &[LogTag<'_>] = if let Some(effect) = ability {
            &[LogTag::From(effect)]
        } else {
            &[]
        };
        // Undefined optional message is an empty protocol argument, as in TS.
        self.add(LogEntry::new("-formechange", &args, tags));
    }
    fn tera_apparent_type(&mut self, mon: MonId) {
        let p = &mut self.state.pokemon[mon.0 as usize];
        if p.terastallized != TypeId::NONE {
            p.flags |= mon_flags::KNOWN_TYPE;
            p.apparent_types = [p.terastallized, TypeId::NONE];
        }
    }
}
