//! Private state helpers shared by the lifecycle modules. OWNER L.
//!
//! Everything here is a plain read/write of Copy state or a thin wrapper around a
//! dispatcher call; nothing draws from the PRNG unless it says so. Method names carry
//! an `lc_` prefix so they cannot collide with another owner's public helpers.

use crate::{
    Battle, dex,
    event::{EffectRef, EventArg, Relay},
    ids::*,
    log::LogSink,
    state::{CellRef, Pokemon, mon_flags},
};

// MoveSlot flag bits (true disable, hidden disable, used, virtual) and the per-turn item flag
// are owned by D; lifecycle clears them at the TS reset points.
pub(super) use crate::actions::mutators::{
    common::USED_ITEM_THIS_TURN,
    pokemon::{SLOT_DISABLED, SLOT_HIDDEN, SLOT_USED},
};

pub(super) const TYPE_ROCK: TypeId = TypeId(6);
pub(super) const TYPE_FIRE: TypeId = TypeId(10);
pub(super) const TYPE_WATER: TypeId = TypeId(11);
pub(super) const TYPE_GRASS: TypeId = TypeId(12);

pub(super) fn mon_arg(m: MonId) -> EventArg {
    EventArg::Holder(Holder::mon(m))
}

/// JS `Pokemon | null`: an absent identity becomes an explicit null.
pub(super) fn opt_mon_arg(m: MonId) -> EventArg {
    if m == MonId::NONE {
        EventArg::Null
    } else {
        mon_arg(m)
    }
}

/// `Effect | null` view of an effect id (the empty id is JS null / the empty effect).
pub(super) fn dex_effect(id: EffectId) -> EffectRef {
    if id == EffectId::NONE {
        EffectRef::None
    } else {
        EffectRef::Dex(id)
    }
}

/// JS number conversion of a relay that the source arithmetic treats as a number.
pub(super) fn relay_number(r: Relay, what: &str) -> f64 {
    match r {
        Relay::Number(n) => n,
        Relay::Bool(b) => f64::from(u8::from(b)),
        other => panic!("{what} produced a non-numeric relay {other:?}"),
    }
}

impl<L: LogSink> Battle<L> {
    #[inline]
    pub(super) fn lc_mon(&self, m: MonId) -> &Pokemon {
        &self.state.pokemon[m.0 as usize]
    }

    #[inline]
    pub(super) fn lc_mon_mut(&mut self, m: MonId) -> &mut Pokemon {
        &mut self.state.pokemon[m.0 as usize]
    }

    #[inline]
    pub(super) fn lc_flag(&self, m: MonId, flag: u32) -> bool {
        self.state.pokemon[m.0 as usize].flags & flag != 0
    }

    #[inline]
    pub(super) fn lc_set_flag(&mut self, m: MonId, flag: u32, on: bool) {
        let f = &mut self.state.pokemon[m.0 as usize].flags;
        if on {
            *f |= flag;
        } else {
            *f &= !flag;
        }
    }

    #[inline]
    pub(super) fn lc_is_active(&self, m: MonId) -> bool {
        self.lc_flag(m, mon_flags::ACTIVE)
    }

    #[inline]
    pub(super) fn lc_is_fainted(&self, m: MonId) -> bool {
        self.lc_flag(m, mon_flags::FAINTED)
    }

    /// JS truthiness of `pokemon.switchFlag` (true or a move-id string).
    /// `SWITCH_REQUESTED` is `true`; a non-empty `switch_flag` is the string form.
    pub(super) fn lc_switch_flag(&self, m: MonId) -> bool {
        let p = self.lc_mon(m);
        p.flags & mon_flags::SWITCH_REQUESTED != 0 || p.switch_flag != EffectId::NONE
    }

    /// `typeof pokemon.switchFlag === 'string'` and its value (U-turn, Baton Pass, ...).
    pub(super) fn lc_switch_flag_move(&self, m: MonId) -> Option<EffectId> {
        let id = self.lc_mon(m).switch_flag;
        (id != EffectId::NONE).then_some(id)
    }

    /// `pokemon.switchFlag = false`.
    pub(super) fn lc_clear_switch_flag(&mut self, m: MonId) {
        let p = self.lc_mon_mut(m);
        p.flags &= !mon_flags::SWITCH_REQUESTED;
        p.switch_flag = EffectId::NONE;
    }

    /// `pokemon.switchFlag = true`.
    pub(super) fn lc_set_switch_flag_true(&mut self, m: MonId) {
        let p = self.lc_mon_mut(m);
        p.flags |= mon_flags::SWITCH_REQUESTED;
        p.switch_flag = EffectId::NONE;
    }

    /// `pokemon.switchFlag === 'revivalblessing'`.
    pub(super) fn lc_switch_flag_is_revival(&self, m: MonId) -> bool {
        self.lc_mon(m).switch_flag == dex::MOVE_REVIVALBLESSING
    }

    /// Pokemon.getFieldPositionValue (pokemon.ts:526-529): side.n + sides.length * position.
    pub(super) fn lc_field_position(&self, m: MonId) -> u8 {
        m.side().0 + 2 * self.lc_mon(m).position
    }

    /// `side.slotConditions[position].revivalblessing` as a direct state read
    /// (side.ts:464-510; sim/battle.ts:2882).
    pub(super) fn lc_has_revival_slot(&self, side: SideId, position: u8) -> bool {
        let list = &self.state.sides[side.0 as usize].slot_conditions[position as usize];
        list.as_slice()
            .iter()
            .any(|c| self.state.effects.cells[c.0 as usize].id == dex::CONDITION_REVIVALBLESSING)
    }

    /// Pokemon.teraType (pokemon.ts:452): `set.teraType || this.types[0]` where `types` is
    /// the base species' types at construction. Immutable for the whole battle.
    pub(super) fn lc_tera_type(&self, m: MonId) -> TypeId {
        let set = &self.teams.sides[m.side().0 as usize].sets[m.0 as usize % 6];
        if set.tera_type != TypeId::NONE {
            set.tera_type
        } else {
            dex::species(set.species).types[0]
        }
    }

    /// `singleEvent('End', pokemon.getAbility(), pokemon.abilityState, pokemon)`.
    /// PRNG: End handlers only.
    pub(super) fn lc_end_ability(&mut self, m: MonId) {
        let (ability, cell) = {
            let p = self.lc_mon(m);
            (p.ability, p.ability_state)
        };
        let state = self.state.effects.capture(cell);
        self.single_event(
            dex::EventId::End,
            dex_effect(ability),
            Some(state),
            mon_arg(m),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            None,
        );
    }

    /// `singleEvent('End', pokemon.getItem(), pokemon.itemState, pokemon)`.
    /// PRNG: End handlers only.
    pub(super) fn lc_end_item(&mut self, m: MonId) {
        let (item, cell) = {
            let p = self.lc_mon(m);
            (p.item, p.item_state)
        };
        let state = self.state.effects.capture(cell);
        self.single_event(
            dex::EventId::End,
            dex_effect(item),
            Some(state),
            mon_arg(m),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            None,
        );
    }

    /// Battle.initEffectState (battle.ts:3320-3330) for `{id, target: mon}`: effectOrder is
    /// `B.effectOrder++` for a non-empty id on an active Pokemon, otherwise zero. Returns a
    /// NEW cell (new object identity); the caller installs it and releases the old one.
    pub(super) fn lc_init_mon_state(&mut self, m: MonId, id: EffectId) -> crate::state::CellId {
        let effect_order = if id != EffectId::NONE && self.lc_is_active(m) {
            let o = self.state.effect_order;
            self.state.effect_order = o.checked_add(1).expect("effectOrder overflow");
            o
        } else {
            0
        };
        self.state
            .effects
            .alloc(Holder::mon(m), Holder::mon(m), id, effect_order)
    }

    /// Capture a cell's identity for `singleEvent(..., state, ...)`.
    pub(super) fn lc_capture(&self, cell: crate::state::CellId) -> CellRef {
        self.state.effects.capture(cell)
    }

    /// Fixed-order walk of the current move slots (aliasing base slots unless Transformed).
    pub(super) fn lc_slot_count(&self, m: MonId) -> usize {
        self.lc_mon(m).move_slots().len()
    }

    pub(super) fn lc_slot_mut(&mut self, m: MonId, i: usize) -> &mut crate::state::MoveSlot {
        let p = self.lc_mon_mut(m);
        if p.flags & mon_flags::TRANSFORMED != 0 {
            &mut p.virtual_move_slots[i]
        } else {
            &mut p.base_move_slots[i]
        }
    }
}
