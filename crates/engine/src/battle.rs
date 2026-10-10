use crate::{
    dex,
    ids::*,
    log::{LogSink, NoLog},
    state::{mon_flags, *},
    teams::{Gender, TeamDef, TeamDefs, TeamError},
};
/// Team definitions are immutable after initialization, shared separately from
/// the Copy simulation snapshot. Battle::new deliberately stops before start().
pub struct Battle<L: LogSink = NoLog> {
    pub state: BattleState,
    pub(crate) teams: TeamDefs,
    pub log: L,
    pub(crate) names: [String; 2],
    pub(crate) scratch: crate::event::Scratch,
}
impl Battle<NoLog> {
    pub fn new(seed: [u16; 4], p1_packed: &str, p2_packed: &str) -> Result<Self, TeamError> {
        Self::with_log(seed, p1_packed, p2_packed, NoLog)
    }
}
impl<L: LogSink> Battle<L> {
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
        let mut teams = TeamDefs {
            sides: [TeamDef::unpack(p1_packed)?, TeamDef::unpack(p2_packed)?],
        };
        let mut state = BattleState::empty(seed);
        for &rule in dex::INITIAL_PSEUDO_WEATHER {
            let cell = state.effects.alloc(Holder::FIELD, Holder::NONE, rule, 0);
            state.field.pseudo_weather.push(cell);
        }
        // Side and Pokemon constructors run sequentially: p1's sets, then p2's.
        for side in 0..2 {
            let team = &mut teams.sides[side];
            state.sides[side].pokemon_count = team.len;
            state.sides[side].pokemon_left = team.len;
            for pos in 0..team.len as usize {
                let mon = MonId((side * 6 + pos) as u8);
                let set = &mut team.sets[pos];
                let s = dex::species(set.species);
                if set.gender == Gender::Unspecified {
                    set.gender = match s.gender {
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
                }
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
                    position: pos as u8,
                    move_count: set.move_count,
                    ..Pokemon::default()
                };
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
                state.pokemon[mon.0 as usize] = p;
                state.sides[side].party[pos] = mon;
            }
        }
        Ok(Self {
            state,
            teams,
            log,
            names: names.map(str::to_owned),
            scratch: crate::event::Scratch::default(),
        })
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
        self.names = [p1.into(), p2.into()];
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
