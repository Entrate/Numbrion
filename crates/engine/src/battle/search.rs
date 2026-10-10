//! Restricted hidden-world import: no live setters, events, logs or PRNG draws.
use super::{Battle, pokemon_from_set};
use crate::{dex::{self, HookRel}, event::EffectRef, ids::*, log::LogSink,
    state::EFFECT_CAPACITY, teams::{Gender, SetDef, TeamError}};
use std::sync::Arc;

fn gender_draw(set: &SetDef) -> bool {
    set.gender == Gender::Unspecified && !matches!(dex::species(set.species).gender, "M" | "F" | "N")
}

impl<L: LogSink> Battle<L> {
    /// Replace an untouched, never-revealed opponent set at a flushed boundary.
    /// `mon` is a stable MonId, not its current party position; `viewer` must be the other side.
    /// Preserve constructor gender-draw count, arena identities, queue, requests and PRNG.
    /// Reject revealed/transformed/damaged/otherwise modified mons and, after start,
    /// species with BattleStart hooks. Errors leave the battle intact. Definitions use COW.
    pub fn replace_unrevealed_set(&mut self, viewer: SideId, mon: MonId, mut set: SetDef) -> Result<(), TeamError> {
        self.assert_boundary();
        if viewer.0 >= 2 || mon.0 >= 12 || mon.side() == viewer
            || !self.state.sides[mon.side().0 as usize].party.contains(&mon)
        {
            return Err(TeamError("Replacement must name a present opponent Pokemon".into()));
        }
        set.validate()?;
        let side = mon.side().0 as usize;
        let index = mon.0 as usize % 6;
        let old = &self.teams.sides[side].sets[index];
        let raw_old = &self.parsed.sides[side].sets[index];
        let p = self.state.pokemon[mon.0 as usize];
        if self.state.revealed_mons & (1 << mon.0) != 0 {
            return Err(TeamError("Cannot replace a Pokemon that has been revealed or switched in".into()));
        }
        if self.state.started && [old.species, set.species].iter().any(|&species|
            self.event_hook(EffectRef::SpeciesCondition(species), EventId::BattleStart, HookRel::On).is_some())
        {
            return Err(TeamError("BattleStart species require construction and history replay".into()));
        }
        if gender_draw(raw_old) != gender_draw(&set) {
            return Err(TeamError("Changing constructor gender draws requires construction and history replay".into()));
        }
        let raw_set = set.clone();
        if set.gender == Gender::Unspecified {
            set.gender = if gender_draw(&set) { old.gender } else {
                match dex::species(set.species).gender {
                    "M" => Gender::Male, "F" => Gender::Female, _ => Gender::Genderless,
                }
            };
        }
        let with_cells = |mut template: crate::state::Pokemon| {
            template.species_state = p.species_state;
            template.status_state = p.status_state;
            template.ability_state = p.ability_state;
            template.item_state = p.item_state;
            template
        };
        if p != with_cells(pokemon_from_set(old, p.position)?) {
            return Err(TeamError("Only untouched, never-revealed Pokemon can be replaced".into()));
        }
        for (cell, target, id) in [
            (p.species_state, Holder::NONE, old.species),
            (p.status_state, Holder::NONE, EffectId::NONE),
            (p.ability_state, Holder::mon(mon), old.ability),
            (p.item_state, Holder::mon(mon), old.item),
        ] {
            if cell.0 as usize >= EFFECT_CAPACITY
                || !self.state.effects.cells[cell.0 as usize].is_initial(Holder::mon(mon), target, id)
            {
                return Err(TeamError("Modified effect state requires construction and history replay".into()));
            }
        }
        let replacement = with_cells(pokemon_from_set(&set, p.position)?);
        let mut teams = Arc::clone(&self.teams);
        Arc::make_mut(&mut Arc::make_mut(&mut teams).sides[side]).sets[index] = set;
        let mut parsed = Arc::clone(&self.parsed);
        Arc::make_mut(&mut Arc::make_mut(&mut parsed).sides[side]).sets[index] = raw_set;
        self.teams = teams;
        self.parsed = parsed;
        self.state.pokemon[mon.0 as usize] = replacement;
        self.state.effects.cells[p.species_state.0 as usize].id = replacement.species;
        self.state.effects.cells[p.ability_state.0 as usize].id = replacement.ability;
        self.state.effects.cells[p.item_state.0 as usize].id = replacement.item;
        self.invalidate_derived_caches();
        Ok(())
    }

    pub(crate) fn mark_revealed(&mut self, mon: MonId) {
        if mon.0 >= 12 { return; }
        self.state.revealed_mons |= 1 << mon.0;
        let illusion = self.state.pokemon[mon.0 as usize].illusion;
        if illusion.0 < 12 {
            self.state.revealed_mons |= 1 << illusion.0;
        }
    }
}
