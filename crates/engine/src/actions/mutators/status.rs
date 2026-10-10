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
        CellId, Status, Trapped, mon_flags, present,
        scratch::{HitData, OrderedBoosts},
    },
};
impl<L: LogSink> Battle<L> {
    /// Immunity/SetStatus/Start/AfterSetStatus with rollback on failed Start
    /// Ports `sim/pokemon.ts:1684-1752`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn set_status(
        &mut self,
        pokemon: MonId,
        status: EffectId,
        attribution: Attribution,
        ignore_immunities: bool,
    ) -> Relay {
        let status = self.condition_id(status);
        let p = &self.state.pokemon[pokemon.0 as usize];
        if p.hp == 0 || (p.flags & mon_flags::ACTIVE == 0 && status != EffectId::NONE) {
            return Relay::Bool(false);
        }
        let mut a = self.mutation_attribution(attribution, false, true);
        if Self::arg_mon(a.source).is_none() {
            a.source = mon_arg(Some(pokemon));
        }
        let move_status = self.effect_status(a.effect);
        if self.status_id(pokemon) == status {
            if move_status != EffectId::NONE {
                if move_status == status {
                    self.add(LogEntry::new(
                        "-fail",
                        &[
                            LogArg::Mon(pokemon),
                            LogArg::Status(self.state.pokemon[pokemon.0 as usize].status),
                        ],
                        &[],
                    ));
                } else {
                    self.add(LogEntry::new(
                        "-fail",
                        &[LogArg::Mon(Self::arg_mon(a.source).unwrap())],
                        &[],
                    ));
                    self.attr_last_move(crate::log::MoveLineEdit::Still);
                }
            }
            return Relay::Bool(false);
        }
        if !ignore_immunities && status != EffectId::NONE {
            let corroded = (status == dex::CONDITION_PSN || status == dex::CONDITION_TOX)
                && self.query_has_ability(Self::arg_mon(a.source).unwrap(), "corrosion");
            let im = match status {
                dex::CONDITION_BRN => ImmunityId::Brn,
                dex::CONDITION_PAR => ImmunityId::Par,
                dex::CONDITION_SLP => {
                    return self.install_status(pokemon, status, a);
                }
                dex::CONDITION_FRZ => ImmunityId::Frz,
                dex::CONDITION_PSN | dex::CONDITION_TOX => ImmunityId::Psn,
                _ => panic!("invalid major status"),
            };
            if !corroded && !self.run_status_immunity(pokemon, im, ImmunityMessage::Silent) {
                if move_status != EffectId::NONE {
                    self.add(LogEntry::new("-immune", &[LogArg::Mon(pokemon)], &[]));
                }
                return Relay::Bool(false);
            }
        }
        // Sleep has no natural immunity but still runs the generic Immunity event.
        self.install_status(pokemon, status, a)
    }
    /// Delegate existing-status-or-requested-status exactly as the source does
    /// Ports `sim/pokemon.ts:1669-1672`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn try_set_status(
        &mut self,
        pokemon: MonId,
        status: EffectId,
        attribution: Attribution,
    ) -> Relay {
        let current = self.status_id(pokemon);
        self.set_status(
            pokemon,
            if current == EffectId::NONE {
                status
            } else {
                current
            },
            attribution,
            false,
        )
    }
    /// Emit cure message then clear_status
    /// Ports `sim/pokemon.ts:1674-1682`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn cure_status(&mut self, pokemon: MonId, silent: bool) -> bool {
        let p = &self.state.pokemon[pokemon.0 as usize];
        if p.hp == 0 || p.status == Status::None {
            return false;
        }
        self.add(LogEntry::new(
            "-curestatus",
            &[LogArg::Mon(pokemon), LogArg::Status(p.status)],
            &[LogTag::Bare(if silent { "silent" } else { "msg" })],
        ));
        self.clear_status(pokemon)
    }
    /// SetStatus empty condition and clear state object
    /// Ports `sim/pokemon.ts:1754-1766`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn clear_status(&mut self, pokemon: MonId) -> bool {
        let p = &self.state.pokemon[pokemon.0 as usize];
        if p.hp == 0 || p.status == Status::None {
            return false;
        }
        // Nightmare is outside the generated closure.
        self.set_status(pokemon, EffectId::NONE, Attribution::NONE, false);
        true
    }
    /// Trapping hypotheses and Ghost immunity
    /// Ports `sim/pokemon.ts:1607-1617`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn try_trap(&mut self, pokemon: MonId, hidden: bool) -> bool {
        if !self.run_status_immunity(pokemon, ImmunityId::Trapped, ImmunityMessage::Silent) {
            return false;
        }
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        if p.trapped != Trapped::No && hidden {
            return true;
        }
        p.trapped = if hidden {
            Trapped::Hidden
        } else {
            Trapped::Yes
        };
        true
    }
    /// Restart existing cell; new cell duration callback, Start and linked cleanup
    /// Ports `sim/pokemon.ts:1969-2027`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn add_volatile(
        &mut self,
        pokemon: MonId,
        status: EffectId,
        attribution: Attribution,
        linked_status: Option<EffectId>,
    ) -> Relay {
        let status = self.condition_id(status);
        let effect = self.condition_ref(status);
        if self.state.pokemon[pokemon.0 as usize].hp == 0
            && dex::effect(status).data.get(dex::FIELD_AFFECTSFAINTED)
                != Some(dex::DataValue::Bool(true))
        {
            return Relay::Bool(false);
        }
        if linked_status.is_some()
            && Self::arg_mon(attribution.source)
                .is_some_and(|m| self.state.pokemon[m.0 as usize].hp == 0)
        {
            return Relay::Bool(false);
        }
        let mut a = self.mutation_attribution(attribution, false, true);
        if Self::arg_mon(a.source).is_none() {
            a.source = mon_arg(Some(pokemon));
        }
        if let Some(cell) = self.get_volatile(pokemon, status) {
            if self
                .event_hook(effect, EventId::Restart, dex::HookRel::On)
                .is_none()
            {
                return Relay::Bool(false);
            }
            return self.mutation_single(EventId::Restart, effect, cell, mon_arg(Some(pokemon)), a);
        }
        if !self.condition_immunity(pokemon, status) {
            if self.effect_status(a.effect) != EffectId::NONE {
                self.add(LogEntry::new("-immune", &[LogArg::Mon(pokemon)], &[]));
            }
            return Relay::Bool(false);
        }
        let r = self.mutation_event(
            EventId::TryAddVolatile,
            mon_arg(Some(pokemon)),
            a,
            Relay::Effect(status),
        );
        if !r.truthy() {
            return r;
        }
        let cell = self.mutation_cell(Holder::mon(pokemon), Holder::mon(pokemon), status);
        self.state.pokemon[pokemon.0 as usize].volatiles.push(cell);
        self.condition_source(cell, a, true, true);
        self.condition_duration(cell, effect, mon_arg(Some(pokemon)), a);
        let r = self.mutation_single(EventId::Start, effect, cell, mon_arg(Some(pokemon)), a);
        if !r.truthy() {
            self.erase_volatile(pokemon, status);
            return r;
        }
        if let Some(linked) = linked_status {
            let source = Self::arg_mon(a.source).unwrap();
            let linked = self.condition_id(linked);
            let other = if let Some(c) = self.get_volatile(source, linked) {
                c
            } else {
                self.add_volatile(
                    source,
                    linked,
                    Attribution::from_move(pokemon, a.effect),
                    None,
                );
                let c = self
                    .get_volatile(source, linked)
                    .expect("linked volatile rejected");
                self.state.effects.cells[c.0 as usize].linked_status = status;
                self.state.effects.cells[c.0 as usize].present |= present::LINKED_STATUS;
                c
            };
            self.state.effects.cells[other.0 as usize].linked_mons |= 1 << pokemon.0;
            self.state.effects.cells[cell.0 as usize].linked_mons = 1 << source.0;
            self.state.effects.cells[cell.0 as usize].linked_status = linked;
            self.state.effects.cells[cell.0 as usize].present |= present::LINKED_STATUS;
        }
        Relay::Bool(true)
    }
    /// Insertion-list lookup; caller can inspect arena cell
    /// Ports `sim/pokemon.ts:2029-2033`. PRNG: none.
    pub fn get_volatile(&self, pokemon: MonId, status: EffectId) -> Option<CellId> {
        self.state
            .pokemon
            .get(pokemon.0 as usize)
            .expect("invalid Pokemon volatile lookup")
            .volatiles
            .as_slice()
            .iter()
            .copied()
            .find(|cell| self.state.effects.cells[cell.0 as usize].id == status)
    }
    /// End then retire/remove captured cell, preserving pins
    /// Ports `sim/pokemon.ts:2035-2046`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn remove_volatile(&mut self, pokemon: MonId, status: EffectId) -> bool {
        if self.state.pokemon[pokemon.0 as usize].hp == 0 {
            return false;
        }
        let status = self.condition_id(status);
        let Some(cell) = self.get_volatile(pokemon, status) else {
            return false;
        };
        let old = self.state.effects.cells[cell.0 as usize];
        let effect = self.condition_ref(status);
        self.mutation_single(
            EventId::End,
            effect,
            cell,
            mon_arg(Some(pokemon)),
            Attribution::NONE,
        );
        self.erase_volatile(pokemon, status);
        self.unlink_volatile(pokemon, old.linked_status, old.linked_mons);
        true
    }
    /// Traverse linked Pokemon bits without changing unrelated list order
    /// Ports `sim/pokemon.ts:2048-2061`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn remove_linked_volatiles(
        &mut self,
        pokemon: MonId,
        status: EffectId,
        linked: EffectId,
    ) -> () {
        let status = self.condition_id(status);
        if let Some(cell) = self.get_volatile(pokemon, status) {
            let mons = self.state.effects.cells[cell.0 as usize].linked_mons;
            self.unlink_volatile(pokemon, linked, mons);
        }
    }
}

impl<L: LogSink> Battle<L> {
    pub(crate) fn condition_immunity(&mut self, mon: MonId, status: EffectId) -> bool {
        if self.state.pokemon[mon.0 as usize].flags & mon_flags::FAINTED != 0 {
            return false;
        }
        // Dex.getImmunity queries types even for an unknown immunity string.
        let _ = self.get_types(mon, false, false);
        self.mutation_event(
            EventId::Immunity,
            mon_arg(Some(mon)),
            Attribution::NONE,
            Relay::Effect(status),
        )
        .truthy()
    }
    fn install_status(&mut self, mon: MonId, status: EffectId, a: Attribution) -> Relay {
        if status == dex::CONDITION_SLP && !self.condition_immunity(mon, status) {
            if self.effect_status(a.effect) != EffectId::NONE {
                self.add(LogEntry::new("-immune", &[LogArg::Mon(mon)], &[]));
            }
            return Relay::Bool(false);
        }
        if status != EffectId::NONE {
            let r = self.mutation_event(
                EventId::SetStatus,
                mon_arg(Some(mon)),
                a,
                Relay::Effect(status),
            );
            if !r.truthy() {
                return r;
            }
        }
        let prev = (
            self.state.pokemon[mon.0 as usize].status,
            self.state.pokemon[mon.0 as usize].status_state,
        );
        let cell = self.mutation_cell(Holder::mon(mon), Holder::mon(mon), status);
        self.state.pokemon[mon.0 as usize].status = match status {
            EffectId::NONE => Status::None,
            dex::CONDITION_BRN => Status::Burn,
            dex::CONDITION_PAR => Status::Paralysis,
            dex::CONDITION_SLP => Status::Sleep,
            dex::CONDITION_FRZ => Status::Freeze,
            dex::CONDITION_PSN => Status::Poison,
            dex::CONDITION_TOX => Status::Toxic,
            _ => panic!("invalid major status"),
        };
        self.state.pokemon[mon.0 as usize].status_state = cell;
        self.condition_source(cell, a, false, false);
        let effect = self.condition_ref(status);
        self.condition_duration(cell, effect, mon_arg(Some(mon)), a);
        if status != EffectId::NONE
            && !self
                .mutation_single(EventId::Start, effect, cell, mon_arg(Some(mon)), a)
                .truthy()
        {
            self.state.pokemon[mon.0 as usize].status = prev.0;
            self.state.pokemon[mon.0 as usize].status_state = prev.1;
            self.release_cell(cell);
            return Relay::Bool(false);
        }
        self.release_cell(prev.1);
        if status != EffectId::NONE
            && !self
                .mutation_event(
                    EventId::AfterSetStatus,
                    mon_arg(Some(mon)),
                    a,
                    Relay::Effect(status),
                )
                .truthy()
        {
            return Relay::Bool(false);
        }
        Relay::Bool(true)
    }
    pub(crate) fn erase_volatile(&mut self, mon: MonId, status: EffectId) {
        if let Some(i) = self.state.pokemon[mon.0 as usize]
            .volatiles
            .as_slice()
            .iter()
            .position(|c| self.state.effects.cells[c.0 as usize].id == status)
        {
            let c = self.state.pokemon[mon.0 as usize].volatiles.remove(i);
            self.release_cell(c);
        }
    }
    pub(crate) fn unlink_volatile(&mut self, mon: MonId, linked: EffectId, mons: u16) {
        for i in 0..12 {
            if mons & (1 << i) == 0 {
                continue;
            }
            let m = MonId(i);
            if let Some(cell) = self.get_volatile(m, linked) {
                self.state.effects.cells[cell.0 as usize].linked_mons &= !(1 << mon.0);
                if self.state.effects.cells[cell.0 as usize].linked_mons == 0 {
                    self.remove_volatile(m, linked);
                }
            }
        }
    }
}
