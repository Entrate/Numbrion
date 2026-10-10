//! Party permutation, initial switches and Tera action. OWNER L.

use crate::{
    Battle,
    actions::FormeOptions,
    dex,
    event::{
        EffectRef, EventArg, EventTargets, Priority, Relay, RunEventOptions, SortOrder,
        SpeedSortable, SyntheticEffect, speed_sort,
    },
    ids::{EffectId, EffectKind, EventId, MonId, SideId, SlotId, TypeId},
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{Status, choices::ActionKind, mon_flags},
};

use super::{
    queue::ActionChoice,
    util::{SLOT_USED, USED_ITEM_THIS_TURN, TYPE_FIRE, TYPE_GRASS, TYPE_ROCK, TYPE_WATER, mon_arg},
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

    fn push(&mut self, m: MonId) {
        self.entries[usize::from(self.len)] = m;
        self.len += 1;
    }
}

/// `switchIn` returns the string 'pursuitfaint' when the outgoing Pokemon is at 0 HP after
/// SwitchOut (battle-actions.ts:98-101); it is truthy and distinct from `true`.
/// Unreachable in this format (Pursuit is not in the Gen 9 dex).
pub const PURSUIT_FAINT: Relay = Relay::PseudoMove(SyntheticEffect::Fainted);

/// `pokemon.speed` sort key for `speedSort(allActive)`: comparePriority on Pokemon objects
/// reads order/priority/subOrder/effectOrder as undefined, i.e. speed descending.
#[derive(Clone, Copy)]
struct ActiveSpeed {
    mon: MonId,
    speed: u16,
}

impl SpeedSortable for ActiveSpeed {
    fn sort_key(&self) -> Priority {
        Priority {
            speed: f64::from(self.speed),
            ..Priority::default()
        }
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
        if pokemon == MonId::NONE || self.lc_is_active(pokemon) {
            self.hint(
                LogArg::Text(
                    "A switch failed because the Pokémon trying to switch in is already in.",
                ),
                false,
                None,
            );
            return Relay::Bool(false);
        }
        let side = pokemon.side();
        let s = usize::from(side.0);
        assert!(position < 2, "Invalid switch position {position} / 2");
        let old_active = self.state.sides[s].active[usize::from(position)];
        let unfainted_active = old_active != MonId::NONE && self.lc_mon(old_active).hp != 0;
        if unfainted_active {
            self.lc_set_flag(old_active, mon_flags::BEING_CALLED_BACK, true);
            // `typeof sourceEffect.selfSwitch === 'string'`: Baton Pass ('copyvolatile') or
            // Shed Tail ('shedtail'); `selfSwitch: true` (U-turn...) is not a string.
            let source_id = self.lc_switch_source_id(source_effect);
            let switch_copy_move = (source_id != EffectId::NONE
                && source_id.kind() == Some(EffectKind::Move)
                && matches!(
                    dex::move_data(source_id).effects.self_switch,
                    dex::SelfSwitch::CopyVolatile | dex::SelfSwitch::ShedTail
                ))
            .then_some(source_id);
            if !self.lc_flag(old_active, mon_flags::SKIP_BEFORE_SWITCH_OUT) && !is_drag {
                self.run_event(
                    EventId::BeforeSwitchOut,
                    mon_arg(old_active),
                    EventArg::Undefined,
                    EffectRef::None,
                    Relay::Undefined,
                    RunEventOptions::default(),
                );
                // gen >= 5
                self.each_event(EventId::Update, EffectRef::None, Relay::Undefined);
            }
            self.lc_set_flag(old_active, mon_flags::SKIP_BEFORE_SWITCH_OUT, false);
            let switch_out = self.run_event(
                EventId::SwitchOut,
                mon_arg(old_active),
                EventArg::Undefined,
                EffectRef::None,
                Relay::Undefined,
                RunEventOptions::default(),
            );
            if !switch_out.truthy() {
                // Warning: DO NOT interrupt a switch-out if you just want to trap a pokemon.
                return Relay::Bool(false);
            }
            if self.lc_mon(old_active).hp == 0 {
                // a pokemon fainted from Pursuit before it could switch
                return PURSUIT_FAINT;
            }

            // will definitely switch out at this point
            self.lc_end_ability(old_active);
            self.lc_end_item(old_active);

            // if a pokemon is forced out by Whirlwind/etc or Eject Button/Pack, it can't
            // use its chosen move
            self.queue_cancel_action(old_active);

            if let Some(move_id) = switch_copy_move {
                self.copy_volatile_from(pokemon, old_active, move_id);
            }
            self.clear_volatile(old_active, true);
        }
        if old_active != MonId::NONE {
            self.lc_set_flag(old_active, mon_flags::ACTIVE, false);
            self.lc_set_flag(old_active, USED_ITEM_THIS_TURN, false);
            self.lc_set_flag(old_active, mon_flags::STATS_RAISED, false);
            self.lc_set_flag(old_active, mon_flags::STATS_LOWERED, false);
            let incoming_position = self.lc_mon(pokemon).position;
            self.lc_mon_mut(old_active).position = incoming_position;
            if self.lc_is_fainted(old_active) {
                self.lc_mon_mut(old_active).status = Status::None;
            }
            self.lc_mon_mut(pokemon).position = position;
            let party = &mut self.state.sides[s].party;
            party[usize::from(position)] = pokemon;
            party[usize::from(incoming_position)] = old_active;
        }
        self.lc_set_flag(pokemon, mon_flags::ACTIVE, true);
        self.mark_revealed(pokemon);
        self.state.sides[s].active[usize::from(position)] = pokemon;
        {
            let p = self.lc_mon_mut(pokemon);
            p.active_turns = 0;
            p.active_move_actions = 0;
        }
        for i in 0..self.lc_slot_count(pokemon) {
            self.lc_slot_mut(pokemon, i).flags &= !SLOT_USED;
        }
        // initEffectState({id: ability, target}) then ({id: item, target}): each advances
        // B.effectOrder for a non-empty id on this now-active Pokemon, and each is a NEW object.
        let (ability, item) = {
            let p = self.lc_mon(pokemon);
            (p.ability, p.item)
        };
        let new_ability = self.lc_init_mon_state(pokemon, ability);
        let old_ability =
            core::mem::replace(&mut self.lc_mon_mut(pokemon).ability_state, new_ability);
        self.state.effects.release(old_ability);
        let new_item = self.lc_init_mon_state(pokemon, item);
        let old_item = core::mem::replace(&mut self.lc_mon_mut(pokemon).item_state, new_item);
        self.state.effects.release(old_item);

        self.run_event(
            EventId::BeforeSwitchIn,
            mon_arg(pokemon),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            RunEventOptions::default(),
        );
        let command = if is_drag { "drag" } else { "switch" };
        let args = [LogArg::Mon(pokemon), LogArg::FullDetails(pokemon)];
        if source_effect != EffectRef::None {
            self.add(LogEntry::split(
                command,
                &args,
                &[LogTag::Value("from", LogArg::Effect(source_effect))],
                side,
                false,
            ));
        } else {
            self.add(LogEntry::split(command, &args, &[], side, false));
        }
        // pokemon.previouslySwitchedIn++ has no reader in this format.

        if is_drag {
            // runSwitch happens immediately so that Mold Breaker can make hazards bypass
            // Clear Body and Levitate
            self.run_switch(pokemon);
        } else {
            self.queue_insert_choice(
                ActionChoice::for_pokemon(ActionKind::RunSwitch, pokemon),
                false,
            );
        }
        Relay::Bool(true)
    }

    /// The move id behind a switch's `sourceEffect` (the dex Move, never an ActiveMove).
    fn lc_switch_source_id(&self, effect: EffectRef) -> EffectId {
        match effect {
            EffectRef::Dex(id) => id,
            EffectRef::ActiveMove(i) => {
                self.scratch.moves[usize::from(i)]
                    .expect("released move frame")
                    .id
            }
            _ => EffectId::NONE,
        }
    }

    /// Select and force a bench replacement; battle-actions.ts:162-173.
    /// PRNG: sample possibleSwitches once when nonempty, including one candidate;
    /// followed by DragOut and switchIn event draws.
    pub fn drag_in(&mut self, side: SideId, position: u8) -> bool {
        let Some(pokemon) = self.get_random_switchable(side) else {
            return false;
        };
        if self.lc_is_active(pokemon) {
            return false;
        }
        let old_active = self.state.sides[usize::from(side.0)].active[usize::from(position)];
        assert!(old_active != MonId::NONE, "nothing to drag out");
        if self.lc_mon(old_active).hp == 0 {
            return false;
        }
        let drag_out = self.run_event(
            EventId::DragOut,
            mon_arg(old_active),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            RunEventOptions::default(),
        );
        if !drag_out.truthy() {
            return false;
        }
        if !self
            .switch_in(pokemon, position, EffectRef::None, true)
            .truthy()
        {
            return false;
        }
        true
    }

    /// Batch consecutive runSwitch actions; battle-actions.ts:175-189.
    /// PRNG: speedSort(allActive,true) tie shuffles then fieldEvent SwitchIn.
    pub fn run_switch(&mut self, pokemon: MonId) {
        let mut switchers = EventTargets {
            mons: [MonId::NONE; 4],
            len: 0,
        };
        switchers.mons[0] = pokemon;
        switchers.len = 1;
        while self
            .queue_peek(false)
            .is_some_and(|a| a.kind == ActionKind::RunSwitch)
        {
            let next = self.queue_shift().expect("peeked action");
            assert!(
                switchers.len < 4,
                "more than four batched runSwitch actions"
            );
            switchers.mons[usize::from(switchers.len)] = next.pokemon;
            switchers.len += 1;
        }
        let all = self.get_all_active(true);
        let mut keyed = [ActiveSpeed {
            mon: MonId::NONE,
            speed: 0,
        }; 4];
        for (i, &m) in all.as_slice().iter().enumerate() {
            keyed[i] = ActiveSpeed {
                mon: m,
                speed: self.lc_mon(m).speed,
            };
        }
        let n = usize::from(all.len);
        speed_sort(&mut self.state.prng, &mut keyed[..n], SortOrder::Priority);
        // battle.speedOrder = allActive.map(a => a.getFieldPositionValue())
        let mut order = self.state.speed_order;
        for (i, k) in keyed[..n].iter().enumerate() {
            order[i] = self.lc_field_position(k.mon);
        }
        self.state.speed_order = order;
        self.field_event(EventId::SwitchIn, Some(switchers));
        // isStarted / draggedIn have no readers in this format.
    }

    /// Enumerate current party order p1 then p2; battle.ts:1361-1367.
    /// PRNG: none; stable MonId identity differs from changing party position.
    pub fn get_all_pokemon(&self) -> PokemonList<12> {
        let mut list = PokemonList::default();
        for side in &self.state.sides {
            for &m in &side.party[..usize::from(side.pokemon_count)] {
                list.push(m);
            }
        }
        list
    }

    /// Enumerate p1a,p1b,p2a,p2b; battle.ts:1369-1379. PRNG: none.
    pub fn get_all_active(&self, include_fainted: bool) -> PokemonList<4> {
        let mut list = PokemonList::default();
        for side in &self.state.sides {
            for &m in &side.active {
                if m != MonId::NONE && (include_fainted || !self.lc_is_fainted(m)) {
                    list.push(m);
                }
            }
        }
        list
    }

    /// Count living bench options; battle.ts:1570-1572. PRNG: none.
    pub fn can_switch(&self, side: SideId) -> u8 {
        self.possible_switches(side).len
    }

    /// Enumerate non-fainted bench in party order; battle.ts:1579-1590.
    /// PRNG: none; no options when side.pokemonLeft is zero.
    pub fn possible_switches(&self, side: SideId) -> PokemonList<6> {
        let mut list = PokemonList::default();
        let s = &self.state.sides[usize::from(side.0)];
        if s.pokemon_left == 0 {
            return list;
        }
        for i in 2..usize::from(s.pokemon_count) {
            let m = s.party[i];
            if !self.lc_is_fainted(m) {
                list.push(m);
            }
        }
        list
    }

    /// Sample a bench option; battle.ts:1574-1577.
    /// PRNG: one sample draw when nonempty, even one candidate; none when empty.
    pub fn get_random_switchable(&mut self, side: SideId) -> Option<MonId> {
        let can_switch_in = self.possible_switches(side);
        if can_switch_in.len == 0 {
            None
        } else {
            Some(self.state.prng.sample(can_switch_in.as_slice()))
        }
    }

    /// Resolve a current active slot; battle.ts:1613-1621.
    /// PRNG: none.
    pub fn get_at_slot(&self, slot: SlotId) -> Option<MonId> {
        if slot == SlotId::NONE {
            return None;
        }
        // positionOffset is zero in doubles; positions past the two active slots are
        // `undefined` array reads.
        let position = usize::from(slot.position());
        if position >= 2 {
            return None;
        }
        let m = self.state.sides[usize::from(slot.side().0)].active[position];
        (m != MonId::NONE).then_some(m)
    }

    /// Request eligibility for this Gen9 format; battle-actions.ts:1911-1916.
    /// PRNG: none; type zero means absent. Preserve cached per-Pokemon eligibility.
    ///
    /// `pokemon.canTerastallize` is `teraType` until the side Terastallizes (then `null` for
    /// every Pokemon of the side, `Side::tera_used`) or the Pokemon transformed into
    /// Ogerpon/Terapagos (`false`, `TERA_BLOCKED`, restored by clearVolatile). Both
    /// non-string states are falsy for every consumer, so they collapse to type zero.
    pub fn can_terastallize(&self, pokemon: MonId) -> TypeId {
        if self.state.sides[usize::from(pokemon.side().0)].tera_used
            || self.lc_flag(pokemon, mon_flags::TERA_BLOCKED)
        {
            return TypeId::NONE;
        }
        self.lc_tera_type(pokemon)
    }

    /// Set Tera state and perform Ogerpon/Terapagos formes;
    /// battle-actions.ts:1918-1955. PRNG: Illusion End, forme/ability and AfterTerastallization events.
    pub fn terastallize(&mut self, pokemon: MonId) {
        let tera_type = self.lc_tera_type(pokemon);
        let illusion = self.lc_mon(pokemon).illusion;
        let illusion_base = (illusion != MonId::NONE)
            .then(|| dex::species(self.lc_mon(illusion).species).base_species_name);
        let species = dex::species(self.lc_mon(pokemon).species);
        if species.base_species_name == "Ogerpon"
            && ![TYPE_FIRE, TYPE_GRASS, TYPE_ROCK, TYPE_WATER].contains(&tera_type)
            && illusion_base.is_none_or(|b| b == "Ogerpon")
        {
            self.hint(
                LogArg::Text(
                    "If Ogerpon Terastallizes into a type other than Fire, Grass, Rock, or Water, the game crashes.",
                ),
                false,
                Some(pokemon.side()),
            );
            return;
        }

        if matches!(illusion_base, Some("Ogerpon" | "Terapagos")) {
            let cell = self.lc_mon(pokemon).ability_state;
            let state = self.lc_capture(cell);
            self.single_event(
                EventId::End,
                EffectRef::Dex(dex::ABILITY_ILLUSION),
                Some(state),
                mon_arg(pokemon),
                EventArg::Undefined,
                EffectRef::None,
                Relay::Undefined,
                None,
            );
        }

        self.add(LogEntry::new(
            "-terastallize",
            &[LogArg::Mon(pokemon), LogArg::Type(tera_type)],
            &[],
        ));
        {
            let p = self.lc_mon_mut(pokemon);
            p.terastallized = tera_type;
            p.added_type = TypeId::NONE;
            p.flags |= mon_flags::KNOWN_TYPE;
            p.apparent_types = [tera_type, TypeId::NONE];
        }
        // for (const ally of pokemon.side.pokemon) ally.canTerastallize = null
        self.state.sides[usize::from(pokemon.side().0)].tera_used = true;

        let species = dex::species(self.lc_mon(pokemon).species);
        if species.base_species_name == "Ogerpon" {
            // toID(species.battleOnly || species.id) + (== 'ogerpon' ? 'tealtera' : 'tera')
            let base = species.battle_only.first().copied().unwrap_or(species.id);
            let target = match base {
                dex::SPECIES_OGERPON => dex::SPECIES_OGERPONTEALTERA,
                dex::SPECIES_OGERPONCORNERSTONE => dex::SPECIES_OGERPONCORNERSTONETERA,
                dex::SPECIES_OGERPONHEARTHFLAME => dex::SPECIES_OGERPONHEARTHFLAMETERA,
                dex::SPECIES_OGERPONWELLSPRING => dex::SPECIES_OGERPONWELLSPRINGTERA,
                other => panic!("no Ogerpon tera forme for {}", dex::effect(other).key),
            };
            self.forme_change(
                pokemon,
                target,
                EffectRef::None,
                FormeOptions {
                    permanent: true,
                    message: None,
                },
            );
        }
        if self.lc_mon(pokemon).species == dex::SPECIES_TERAPAGOSTERASTAL {
            self.forme_change(
                pokemon,
                dex::SPECIES_TERAPAGOSSTELLAR,
                EffectRef::None,
                FormeOptions {
                    permanent: true,
                    message: None,
                },
            );
        }
        let (species_id, base_species_id, transformed) = {
            let p = self.lc_mon(pokemon);
            (
                p.species,
                p.base_species,
                p.flags & mon_flags::TRANSFORMED != 0,
            )
        };
        if dex::species(species_id).base_species_name == "Morpeko"
            && !transformed
            && base_species_id != species_id
        {
            let p = self.lc_mon_mut(pokemon);
            p.flags |= mon_flags::FORME_REGRESSION;
            p.base_species = species_id;
            // pokemon.details = pokemon.getUpdatedDetails(): derived from baseSpecies.
        }
        self.run_event(
            EventId::AfterTerastallization,
            mon_arg(pokemon),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            RunEventOptions::default(),
        );
    }
}
