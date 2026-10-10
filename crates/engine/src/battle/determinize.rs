//! Determinization for search: swap the set of an opponent Pokemon the searching side has never
//! observed, in place, at a decision boundary. Not a Showdown port: no live setters, events, log
//! lines or PRNG draws. Contract on `Battle::replace_hidden_set`; reference test in
//! tests/determinize.rs and `difftest`'s `determinize_check`.
use super::{Battle, pokemon_from_set};
use crate::{
    dex::{self, HookRel},
    event::EffectRef,
    ids::*,
    log::LogSink,
    state::EFFECT_CAPACITY,
    teams::{Gender, SetDef, TeamError},
};
use std::sync::Arc;

/// Why `replace_hidden_set` refused. The battle is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HiddenSetError {
    /// `viewer` is not 0/1, or `mon` is not a party member of `viewer`'s opponent.
    NotOpponent,
    /// The battle is over.
    Ended,
    /// The Pokemon was switched in at some point, or Illusion chose or inspected it.
    Revealed,
    /// Battle code changed the Pokemon or its effect cells after construction.
    Modified,
    /// After start: the old or new species has a BattleStart forme hook (Zacian, Zamazenta).
    /// `start` already ran it for the old set and cannot be replayed for the new one.
    BattleStartSpecies,
    /// The constructor would draw the new set's gender but drew none for the replaced input
    /// (or the input had an explicit gender); give the gender explicitly.
    GenderRequired,
    /// The set fails `SetDef` validation or overflows the stat range.
    InvalidSet(TeamError),
}

impl core::fmt::Display for HiddenSetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotOpponent => "the Pokemon is not a party member of the viewer's opponent",
            Self::Ended => "the battle has ended",
            Self::Revealed => "the Pokemon has been revealed (switched in, or chosen by Illusion)",
            Self::Modified => "the Pokemon or its effect state changed since construction",
            Self::BattleStartSpecies => {
                "Zacian/Zamazenta-style BattleStart species cannot be swapped after start"
            }
            Self::GenderRequired => {
                "the new set needs an explicit gender: the replaced input drew none at construction"
            }
            Self::InvalidSet(e) => return write!(f, "invalid set: {e}"),
        })
    }
}

impl std::error::Error for HiddenSetError {}

/// The constructor samples this set's gender (battle.rs `initialize`).
fn draws_gender(set: &SetDef) -> bool {
    set.gender == Gender::Unspecified
        && !matches!(dex::species(set.species).gender, "M" | "F" | "N")
}

impl<L: LogSink> Battle<L> {
    /// Record that `mon` may have been observed. PRNG: none; search bookkeeping only.
    #[inline]
    pub(crate) fn mark_revealed(&mut self, mon: MonId) {
        self.state.revealed_mons |= 1 << mon.0;
    }

    /// Bit i: MonId(i) was switched in at some point, or Illusion chose or inspected it as a
    /// disguise. Both sides; monotone; carried by snapshots. PRNG: none.
    pub fn revealed_mons(&self) -> u16 {
        self.state.revealed_mons
    }

    /// Bit i: MonId(i) belongs to `viewer`'s opponent and `replace_hidden_set` would accept a
    /// valid set for it now (set-dependent checks aside). PRNG: none.
    pub fn hidden_mons(&self, viewer: SideId) -> u16 {
        if viewer.0 >= 2 {
            return 0;
        }
        let side = &self.state.sides[1 - viewer.0 as usize];
        side.party[..side.pokemon_count as usize]
            .iter()
            .filter(|&&m| self.check_hidden(viewer, m).is_ok())
            .fold(0, |mask, &m| mask | 1 << m.0)
    }

    /// Determinization: give `mon`, a Pokemon of `viewer`'s opponent that was never revealed,
    /// the set `set`, without running battle code. `mon` is the stable MonId (team order), not
    /// the current party position. Requires a flushed decision boundary like `restore_from`.
    ///
    /// Accepted when (else `Err`, battle unchanged): the battle has not ended; `mon` was never
    /// switched in (so never active, fainted, transformed into, Commander-linked, ...) and never
    /// chosen or inspected by Illusion (`revealed_mons`); its `Pokemon` and its four construction
    /// effect cells (species, status, ability, item) are exactly what construction produced,
    /// party position aside; after start, neither species has a BattleStart hook; the gender
    /// rule below; `set` validates.
    ///
    /// Effect: the resolved and the unresolved (`reset_seed` input) definitions take the set,
    /// copy-on-write, so battles sharing the old Arcs are unaffected (workers must then adopt the
    /// new context with `clone_from` before `restore_from`). The Pokemon is rebuilt by the
    /// constructor's `pokemon_from_set` at its current party position (stats, HP, types, weight,
    /// moves/PP, ability, item, species); its effect cells keep their indices and generations and
    /// only change ids. No events, logs or PRNG draws; requests are built on demand.
    ///
    /// Gender: a set with unspecified gender of a random-gender species is resolved by a
    /// constructor draw. Here, when the replaced unresolved input also drew, the new set takes
    /// that drawn gender (what the same position's draw yields); when it did not draw,
    /// `GenderRequired`. Explicit genders and fixed-gender species never draw. The resolved gender
    /// of the real Pokemon is therefore reused; pass an explicit gender to avoid that.
    ///
    /// Reference (tests/determinize.rs, difftest determinize_check): let the battle have come
    /// from construction plus choice history H. The result equals, in `BattleState` and resolved
    /// definitions, `from_team_defs(s0, T')` started and replayed through H, where T' is the
    /// resolved teams with this set (resolved) swapped in and s0 the PRNG right after the
    /// original construction; that replay reproduces the original log, requests of the viewer
    /// and PRNG at every boundary. If the gender-draw count is unchanged at this position, the
    /// same holds for construction from the original seed and the updated unresolved inputs.
    /// When it changes, that same-seed battle shifts every later draw and diverges; search
    /// worlds should be reseeded anyway, the seed being hidden information.
    pub fn replace_hidden_set(
        &mut self,
        viewer: SideId,
        mon: MonId,
        set: &SetDef,
    ) -> Result<(), HiddenSetError> {
        self.assert_boundary();
        self.check_hidden(viewer, mon)?;
        set.validate().map_err(HiddenSetError::InvalidSet)?;
        if self.battle_start_species(set.species) {
            return Err(HiddenSetError::BattleStartSpecies);
        }
        let (side, index) = (mon.side().0 as usize, mon.0 as usize % 6);
        let mut resolved = set.clone();
        if set.gender == Gender::Unspecified {
            resolved.gender = match dex::species(set.species).gender {
                "M" => Gender::Male,
                "F" => Gender::Female,
                "N" => Gender::Genderless,
                _ if draws_gender(&self.parsed.sides[side].sets[index]) => {
                    self.teams.sides[side].sets[index].gender
                }
                _ => return Err(HiddenSetError::GenderRequired),
            };
        }
        let old = self.state.pokemon[mon.0 as usize];
        let mut p = pokemon_from_set(&resolved, old.position).map_err(HiddenSetError::InvalidSet)?;
        p.species_state = old.species_state;
        p.status_state = old.status_state;
        p.ability_state = old.ability_state;
        p.item_state = old.item_state;
        // Nothing can fail below.
        let cells = &mut self.state.effects.cells;
        cells[p.species_state.0 as usize].id = p.species;
        cells[p.ability_state.0 as usize].id = p.ability;
        cells[p.item_state.0 as usize].id = p.item;
        self.state.pokemon[mon.0 as usize] = p;
        // A side whose inputs needed no gender draw shares one TeamDef for both views.
        let shared = Arc::ptr_eq(&self.teams.sides[side], &self.parsed.sides[side]);
        let raw_is_resolved = set.gender == resolved.gender;
        Arc::make_mut(&mut Arc::make_mut(&mut self.teams).sides[side]).sets[index] = resolved;
        let parsed = &mut Arc::make_mut(&mut self.parsed).sides[side];
        if shared && raw_is_resolved {
            *parsed = Arc::clone(&self.teams.sides[side]);
        } else {
            Arc::make_mut(parsed).sets[index] = set.clone();
        }
        self.invalidate_derived_caches();
        Ok(())
    }

    /// Set-independent preconditions of `replace_hidden_set`. PRNG: none.
    fn check_hidden(&self, viewer: SideId, mon: MonId) -> Result<(), HiddenSetError> {
        if viewer.0 >= 2
            || mon.0 >= 12
            || mon.side() == viewer
            || mon.0 as usize % 6 >= self.state.sides[mon.side().0 as usize].pokemon_count as usize
        {
            return Err(HiddenSetError::NotOpponent);
        }
        if self.state.ended {
            return Err(HiddenSetError::Ended);
        }
        if self.state.revealed_mons & (1 << mon.0) != 0 {
            return Err(HiddenSetError::Revealed);
        }
        let old = &self.teams.sides[mon.side().0 as usize].sets[mon.0 as usize % 6];
        if self.battle_start_species(old.species) {
            return Err(HiddenSetError::BattleStartSpecies);
        }
        let p = self.state.pokemon[mon.0 as usize];
        let Ok(mut fresh) = pokemon_from_set(old, p.position) else {
            return Err(HiddenSetError::Modified);
        };
        fresh.species_state = p.species_state;
        fresh.status_state = p.status_state;
        fresh.ability_state = p.ability_state;
        fresh.item_state = p.item_state;
        let owner = Holder::mon(mon);
        let untouched = p == fresh
            && [
                (p.species_state, Holder::NONE, old.species),
                (p.status_state, Holder::NONE, EffectId::NONE),
                (p.ability_state, owner, old.ability),
                (p.item_state, owner, old.item),
            ]
            .iter()
            .all(|&(cell, target, id)| {
                (cell.0 as usize) < EFFECT_CAPACITY
                    && self.state.effects.cells[cell.0 as usize].is_initial(owner, target, id)
            });
        if !untouched {
            return Err(HiddenSetError::Modified);
        }
        Ok(())
    }

    /// `start` runs every Pokemon's species-condition BattleStart hook (turn.rs
    /// `run_start_action`); one that exists cannot be re-run for a swapped set. PRNG: none.
    fn battle_start_species(&self, species: EffectId) -> bool {
        self.state.started
            && self
                .event_hook(
                    EffectRef::SpeciesCondition(species),
                    EventId::BattleStart,
                    HookRel::On,
                )
                .is_some()
    }
}

/// `reveal-audit`: every Pokemon a protocol line names (and the member an Illusion user
/// displays) must already be marked. Diagnostic only; replay corpora with the feature on.
#[cfg(feature = "reveal-audit")]
impl<L: LogSink> Battle<L> {
    fn audit_mon(&self, what: &str, mon: MonId) {
        if mon.0 >= 12 {
            return;
        }
        let illusion = self.state.pokemon[mon.0 as usize].illusion;
        for m in [mon, illusion] {
            assert!(
                m.0 >= 12 || self.state.revealed_mons & (1 << m.0) != 0,
                "reveal-audit: {what} names unmarked MonId({}) on turn {}",
                m.0,
                self.state.turn
            );
        }
    }

    fn audit_arg(&self, what: &str, arg: crate::log::LogArg<'_>) {
        use crate::log::LogArg;
        match arg {
            LogArg::Mon(m) | LogArg::Health(m) | LogArg::Details(m) | LogArg::FullDetails(m) => {
                self.audit_mon(what, m)
            }
            LogArg::Parts(parts) => parts.iter().for_each(|&a| self.audit_arg(what, a)),
            LogArg::Spread { mons, len } => {
                mons[..len as usize].iter().for_each(|&m| self.audit_mon(what, m))
            }
            _ => {}
        }
    }

    fn audit_tag(&self, what: &str, tag: crate::log::LogTag<'_>) {
        use crate::log::LogTag;
        match tag {
            LogTag::Of(m) => self.audit_mon(what, m),
            LogTag::Value(_, arg) => self.audit_arg(what, arg),
            _ => {}
        }
    }

    pub(crate) fn audit_entry(&self, entry: crate::log::LogEntry<'_>) {
        entry.args.iter().for_each(|&a| self.audit_arg(entry.command, a));
        entry.tags.iter().for_each(|&t| self.audit_tag(entry.command, t));
    }

    pub(crate) fn audit_edit(&self, edit: crate::log::MoveLineEdit<'_>) {
        use crate::log::MoveLineEdit;
        match edit {
            MoveLineEdit::Retarget(m) => self.audit_mon("move retarget", m),
            MoveLineEdit::Tag(tag) => self.audit_tag("move tag", tag),
            _ => {}
        }
    }
}
