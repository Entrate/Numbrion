use crate::{
    dex,
    ids::*,
    log::{LogSink, NoLog},
    state::{mon_flags, *},
    teams::{Gender, TeamDef, TeamDefs, TeamError},
};
use std::sync::Arc;
/// Team definitions are immutable after initialization, shared separately from
/// the Copy simulation snapshot. Battle::new deliberately stops before start().
pub struct Battle<L: LogSink = NoLog> {
    pub state: BattleState,
    pub(crate) teams: Arc<TeamDefs>,
    /// Unresolved input retained so reset repeats the constructor's gender draws.
    pub(crate) parsed: Arc<TeamDefs>,
    pub log: L,
    pub(crate) names: Arc<[String; 2]>,
    pub(crate) scratch: crate::event::Scratch,
}
impl Battle<NoLog> {
    pub fn new(seed: [u16; 4], p1_packed: &str, p2_packed: &str) -> Result<Self, TeamError> {
        Self::with_log(seed, p1_packed, p2_packed, NoLog)
    }
}
impl<L: LogSink> Battle<L> {
    /// Derived caches must be invalidated after whole-state writes.
    pub fn invalidate_derived_caches(&mut self) {}

    /// Require a flushed decision/end boundary, or an untouched pre-start battle.
    pub(crate) fn assert_boundary(&self) {
        self.scratch.assert_idle();
        assert!(
            matches!(
                self.state.phase,
                Phase::Created | Phase::Choices | Phase::Ended
            ),
            "snapshot requires a decision boundary"
        );
        assert_eq!(
            self.scratch.unsent_lines,
            if self.state.started { 0 } else { 4 },
            "snapshot requires flushed log updates"
        );
    }

    /// Restore a root into an existing worker with matching resolved teams and names.
    /// Both battles must be at flushed boundaries (including mid-turn replacements).
    /// Retains scratch capacity, clears branch logs, copies the PRNG without draws.
    pub fn restore_from(&mut self, other: &Battle<L>) {
        self.assert_boundary();
        other.assert_boundary();
        // Workers cloned from the root share these Arcs; compare contents otherwise.
        assert!(
            Arc::ptr_eq(&self.teams, &other.teams) || self.teams == other.teams,
            "restore requires matching resolved teams"
        );
        assert!(
            Arc::ptr_eq(&self.parsed, &other.parsed) || self.parsed == other.parsed,
            "restore requires matching parsed teams"
        );
        assert!(
            Arc::ptr_eq(&self.names, &other.names) || self.names == other.names,
            "restore requires matching player names"
        );
        self.state = other.state;
        self.scratch.reset(self.state.started);
        self.log.reset();
        self.invalidate_derived_caches();
    }

    pub fn teams(&self) -> &TeamDefs {
        &self.teams
    }
    pub fn with_log(
        seed: [u16; 4],
        p1_packed: &str,
        p2_packed: &str,
        log: L,
    ) -> Result<Self, TeamError> {
        Self::with_log_named(seed, p1_packed, p2_packed, ["Player 1", "Player 2"], log)
    }
    fn with_log_named(
        seed: [u16; 4],
        p1_packed: &str,
        p2_packed: &str,
        names: [&str; 2],
        log: L,
    ) -> Result<Self, TeamError> {
        Self::from_team_defs(
            seed,
            Arc::new(TeamDef::unpack(p1_packed)?),
            Arc::new(TeamDef::unpack(p2_packed)?),
            names.map(str::to_owned),
            log,
        )
    }

    /// Pre-start construction from parsed inputs; inputs remain unresolved and reusable.
    /// PRNG: p1's unspecified genders in set order, then p2's, exactly as packed construction.
    pub fn from_team_defs(
        seed: [u16; 4],
        p1: Arc<TeamDef>,
        p2: Arc<TeamDef>,
        names: [String; 2],
        log: L,
    ) -> Result<Self, TeamError> {
        let parsed = Arc::new(TeamDefs { sides: [p1, p2] });
        let (state, teams) = Self::initialize(seed, &parsed)?;
        Ok(Self {
            state,
            teams,
            parsed,
            log,
            names: Arc::new(names),
            scratch: crate::event::Scratch::default(),
        })
    }

    fn initialize(
        seed: [u16; 4],
        parsed: &Arc<TeamDefs>,
    ) -> Result<(BattleState, Arc<TeamDefs>), TeamError> {
        for team in &parsed.sides {
            team.validate()?;
        }
        let mut teams = Arc::clone(parsed);
        let mut state = BattleState::empty(seed);
        for &rule in dex::INITIAL_PSEUDO_WEATHER {
            let cell = state.effects.alloc(Holder::FIELD, Holder::NONE, rule, 0);
            state.field.pseudo_weather.push(cell);
        }
        // Side and Pokemon constructors run sequentially: p1's sets, then p2's.
        for side in 0..2 {
            let len = teams.sides[side].len;
            state.sides[side].pokemon_count = len;
            state.sides[side].pokemon_left = len;
            for pos in 0..len as usize {
                let mon = MonId((side * 6 + pos) as u8);
                if teams.sides[side].sets[pos].gender == Gender::Unspecified {
                    let gender = match dex::species(teams.sides[side].sets[pos].species).gender {
                        "M" => Gender::Male,
                        "F" => Gender::Female,
                        "N" => Gender::Genderless,
                        _ => {
                            if state.prng.sample_index(2) == 0 {
                                Gender::Male
                            } else {
                                Gender::Female
                            }
                        }
                    };
                    Arc::make_mut(&mut Arc::make_mut(&mut teams).sides[side]).sets[pos].gender =
                        gender;
                }
                let set = &teams.sides[side].sets[pos];
                let mut p = pokemon_from_set(set, pos as u8)?;
                p.species_state =
                    state
                        .effects
                        .alloc(Holder::mon(mon), Holder::NONE, set.species, 0);
                p.status_state =
                    state
                        .effects
                        .alloc(Holder::mon(mon), Holder::NONE, EffectId::NONE, 0);
                p.ability_state =
                    state
                        .effects
                        .alloc(Holder::mon(mon), Holder::mon(mon), set.ability, 0);
                p.item_state = state
                    .effects
                    .alloc(Holder::mon(mon), Holder::mon(mon), set.item, 0);
                state.pokemon[mon.0 as usize] = p;
                state.sides[side].party[pos] = mon;
            }
        }
        Ok((state, teams))
    }

    /// Reset to Created with new packed teams, preserving names and all scratch capacity.
    pub fn reset(&mut self, seed: [u16; 4], p1: &str, p2: &str) -> Result<(), TeamError> {
        self.reset_from_team_defs(
            seed,
            Arc::new(TeamDef::unpack(p1)?),
            Arc::new(TeamDef::unpack(p2)?),
        )
    }

    /// Reset using parsed inputs. Errors leave the current battle intact. PRNG: constructor draws only.
    pub fn reset_from_team_defs(
        &mut self,
        seed: [u16; 4],
        p1: Arc<TeamDef>,
        p2: Arc<TeamDef>,
    ) -> Result<(), TeamError> {
        self.assert_boundary();
        let parsed =
            if Arc::ptr_eq(&self.parsed.sides[0], &p1) && Arc::ptr_eq(&self.parsed.sides[1], &p2) {
                Arc::clone(&self.parsed)
            } else {
                Arc::new(TeamDefs { sides: [p1, p2] })
            };
        let (state, teams) = Self::initialize(seed, &parsed)?;
        self.state = state;
        self.teams = teams;
        self.parsed = parsed;
        self.scratch.reset(false);
        self.log.reset();
        self.invalidate_derived_caches();
        Ok(())
    }

    /// Reset with the original unresolved teams, so unspecified genders are sampled anew.
    pub fn reset_seed(&mut self, seed: [u16; 4]) -> Result<(), TeamError> {
        self.reset_from_team_defs(
            seed,
            Arc::clone(&self.parsed.sides[0]),
            Arc::clone(&self.parsed.sides[1]),
        )
    }
    /// Named pre-start constructor for the difftest adapter (battle.ts:3224-3257).
    /// PRNG: only the existing constructor's gender samples; names draw nothing.
    pub fn from_players(
        seed: [u16; 4],
        p1: (&str, &str),
        p2: (&str, &str),
        log: L,
    ) -> Result<Self, TeamError> {
        Self::with_log_named(seed, p1.1, p2.1, [p1.0, p2.0], log)
    }
    /// Ports battle.ts:3224-3257. PRNG: none. Set before start; strings stay outside state.
    pub fn set_names(&mut self, p1: &str, p2: &str) {
        assert_eq!(
            self.state.phase,
            Phase::Created,
            "names must be set before start"
        );
        self.names = Arc::new([p1.into(), p2.into()]);
    }
    /// Ports side.ts:353. PRNG: none. Protocol/request boundary metadata.
    pub fn player_name(&self, side: SideId) -> &str {
        &self.names[side.0 as usize]
    }
    /// Ports battle.ts:346. PRNG: none; observation only.
    pub fn seed(&self) -> [u16; 4] {
        self.state.prng.seed()
    }
    /// Ports battle.ts:1627. PRNG: none; observation only.
    pub fn turn(&self) -> u32 {
        self.state.turn as u32
    }
    /// Formatting is a boundary operation. No String lives in BattleState.
    pub fn details(&self, mon: MonId) -> String {
        let p = &self.state.pokemon[mon.0 as usize];
        let set = &self.teams.sides[mon.side().0 as usize].sets[mon.0 as usize % 6];
        let species = dex::species(p.base_species);
        let name = if species.name == "Greninja-Bond" || species.name == "Rockruff-Dusk" {
            species.base_species_name
        } else {
            species.name
        };
        let mut result = name.to_owned();
        if set.level != 100 {
            result.push_str(&format!(", L{}", set.level));
        }
        if !set.gender.protocol().is_empty() {
            result.push_str(", ");
            result.push_str(set.gender.protocol());
        }
        if set.shiny {
            result.push_str(", shiny");
        }
        result
    }
}

/// Independent worker at the same boundary and PRNG, with empty branch logs.
/// `clone` allocates scratch; `clone_from` adopts the context and reuses existing scratch.
impl<L: LogSink + Default> Clone for Battle<L> {
    fn clone(&self) -> Self {
        self.assert_boundary();
        let mut result = Self {
            state: self.state,
            teams: Arc::clone(&self.teams),
            parsed: Arc::clone(&self.parsed),
            names: Arc::clone(&self.names),
            log: L::default(),
            scratch: crate::event::Scratch::default(),
        };
        result.scratch.reset(result.state.started);
        result.invalidate_derived_caches();
        result
    }

    fn clone_from(&mut self, source: &Self) {
        self.assert_boundary();
        source.assert_boundary();
        self.teams = Arc::clone(&source.teams);
        self.parsed = Arc::clone(&source.parsed);
        self.names = Arc::clone(&source.names);
        self.state = source.state;
        self.scratch.reset(self.state.started);
        self.log.reset();
        self.invalidate_derived_caches();
    }
}

pub(crate) fn pokemon_from_set(
    set: &crate::teams::SetDef,
    position: u8,
) -> Result<Pokemon, TeamError> {
    let s = dex::species(set.species);
    let stats = compute_stats(s, set)?;
    let mut p = Pokemon {
        species: set.species,
        base_species: set.species,
        ability: set.ability,
        base_ability: set.ability,
        item: set.item,
        flags: mon_flags::KNOWN_TYPE | mon_flags::NEWLY_SWITCHED,
        types: s.types,
        apparent_types: s.types,
        weighthg: s.weighthg,
        hp: stats[0],
        max_hp: stats[0],
        stored_stats: stats[1..].try_into().unwrap(),
        base_stored_stats: stats[1..].try_into().unwrap(),
        speed: stats[5],
        position,
        move_count: set.move_count,
        ..Pokemon::default()
    };
    for i in 0..set.move_count as usize {
        let m = dex::move_data(set.moves[i]);
        let pp = if m.no_pp_boosts {
            m.pp
        } else {
            (m.pp as u16 * 8 / 5) as u8
        };
        p.base_move_slots[i] = MoveSlot {
            id: m.id,
            pp,
            max_pp: pp,
            target: m.target,
            ..MoveSlot::default()
        };
    }
    Ok(p)
}
/// Shared constructor/form stat calculation (battle.ts:2350-2376). PRNG: none.
pub(crate) fn compute_stats(
    s: &dex::SpeciesData,
    set: &crate::teams::SetDef,
) -> Result<[u16; 6], TeamError> {
    let mut result = [0; 6];
    for (i, out) in result.iter_mut().enumerate() {
        let base = s.base_stats[i] as u32;
        let iv = set.ivs[i] as u32;
        let ev = set.evs[i] as u32 / 4;
        let level = set.level as u32;
        let mut stat = if i == 0 {
            (2 * base + iv + ev + 100) * level / 100 + 10
        } else {
            (2 * base + iv + ev) * level / 100 + 5
        };
        if i == 0 && s.max_hp != 0 {
            stat = s.max_hp as u32
        }
        if i != 0 && set.nature >= 0 {
            let nature = &dex::NATURES[set.nature as usize];
            if nature.plus == i as i8 {
                stat = ((stat * 110) & 0xffff) / 100
            } else if nature.minus == i as i8 {
                stat = ((stat * 90) & 0xffff) / 100
            }
        }
        *out = stat
            .try_into()
            .map_err(|_| TeamError("Stats exceed this format's u16 state capacity".into()))?;
    }
    Ok(result)
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    use crate::{
        actions::MoveHandle,
        log::{LogArg, LogEntry, TextLog},
    };

    const TEAM: &str = concat!(
        "Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric]",
        "Raichu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric"
    );

    #[test]
    fn restore_retains_allocations_and_resets_branch_logging() {
        let mut root = Battle::with_log([1, 2, 3, 4], TEAM, TEAM, TextLog::default()).unwrap();
        root.start().unwrap();
        let mut worker = Battle::with_log([1, 2, 3, 4], TEAM, TEAM, TextLog::default()).unwrap();
        let allocations: Vec<_> = worker
            .scratch
            .handlers
            .iter()
            .map(|h| (h.entries.as_ptr(), h.entries.capacity()))
            .collect();
        worker.restore_from(&root);
        worker.choose(0, "default").unwrap();
        worker.choose(1, "default").unwrap();
        // Completed execution may leave stale, non-live context; reset all of it.
        worker.scratch.active_move = MoveHandle(0);
        worker.scratch.active_pokemon = MonId(0);
        worker.scratch.initial_modifier = 123;
        worker.restore_from(&root);
        assert!(worker.log.entries.is_empty());
        assert_eq!(worker.log.last_move_line, None);
        assert_eq!(worker.log.drain_cursor, 0);
        assert_eq!(worker.scratch.unsent_lines, 0);
        assert_eq!(worker.scratch.active_move, MoveHandle::NONE);
        assert_eq!(worker.scratch.active_pokemon, MonId::NONE);
        assert_eq!(worker.scratch.initial_modifier, 0);
        assert_eq!(worker.scratch.current_frame, 255);
        assert!(worker.scratch.moves.iter().all(Option::is_none));
        assert_eq!(
            allocations,
            worker
                .scratch
                .handlers
                .iter()
                .map(|h| (h.entries.as_ptr(), h.entries.capacity()))
                .collect::<Vec<_>>()
        );
        assert_eq!(worker.seed(), root.seed());
        assert_eq!(worker.request_json(0), root.request_json(0));
    }

    #[test]
    #[should_panic(expected = "snapshot requires flushed log updates")]
    fn restore_rejects_unflushed_source() {
        let mut root = Battle::new([1, 2, 3, 4], TEAM, TEAM).unwrap();
        let mut worker = Battle::new([1, 2, 3, 4], TEAM, TEAM).unwrap();
        root.start().unwrap();
        root.add(LogEntry::new("hint", &[LogArg::Text("unflushed")], &[]));
        worker.restore_from(&root);
    }

    #[test]
    fn clone_and_reset_share_definitions_and_reuse_worker_allocations() {
        let mut root = Battle::new([1, 2, 3, 4], TEAM, TEAM).unwrap();
        root.start().unwrap();
        let mut worker = root.clone();
        assert!(Arc::ptr_eq(&worker.teams, &root.teams));
        assert!(Arc::ptr_eq(&worker.parsed, &root.parsed));
        assert!(Arc::ptr_eq(&worker.names, &root.names));
        let allocations: Vec<_> = worker
            .scratch
            .handlers
            .iter()
            .map(|h| (h.entries.as_ptr(), h.entries.capacity()))
            .collect();
        assert_ne!(allocations[0].0, root.scratch.handlers[0].entries.as_ptr());
        worker.reset_seed([4, 3, 2, 1]).unwrap();
        let fresh = Battle::new([4, 3, 2, 1], TEAM, TEAM).unwrap();
        assert_eq!(format!("{:?}", worker.state), format!("{:?}", fresh.state));
        assert_eq!(worker.scratch.unsent_lines, 4);
        worker.clone_from(&root);
        assert_eq!(worker.seed(), root.seed());
        assert_eq!(
            allocations,
            worker
                .scratch
                .handlers
                .iter()
                .map(|h| (h.entries.as_ptr(), h.entries.capacity()))
                .collect::<Vec<_>>()
        );
    }
}
