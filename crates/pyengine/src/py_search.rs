//! Python bindings of the opt-in fair-information search support (`--features search`):
//! `SetPool`, `RevealTracker` and `WorldBuilder` (engine `search` module,
//! docs/design/SEARCH-PREREQUISITES.md). Worlds come back as ordinary `Battle`s with `is_world`.
use crate::{
    action::pair_text,
    game::{Game, Sink},
    pool::TeamPool,
    py::{Battle, lock},
};
use engine::{
    dex,
    ids::{EffectId, SideId, TypeId},
    log::{NoLog, TextLog},
    search::{
        reveal::{ItemKnowledge, RevealTracker as Tracker},
        sets::SetPool as Pool,
        world::{WorldBuilder as Builder, WorldConfig},
    },
    teams::{Gender, TeamDef},
};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// Generated random-battle sets to sample opponent hypotheses from.
///
/// `SetPool(team_pool_paths=(), max_teams_per_file=None, teams=None)`: packed-team files (`.txt` / `.txt.gz`,
/// one team per line, e.g. `data/teams/pool-s1-200k.txt.gz`) and/or packed teams. Identical sets are stored
/// once with a count, so sampling follows the generator's empirical distribution.
#[pyclass(frozen, module = "numbrion")]
pub(crate) struct SetPool {
    pool: Arc<Pool>,
}

#[pymethods]
impl SetPool {
    #[new]
    #[pyo3(signature = (team_pool_paths=Vec::new(), max_teams_per_file=None, teams=None))]
    fn new(py: Python<'_>, team_pool_paths: Vec<PathBuf>, max_teams_per_file: Option<usize>, teams: Option<Vec<String>>) -> PyResult<Self> {
        let pool = py.detach(|| -> Result<Pool, String> {
            let mut lines = TeamPool::default();
            for p in &team_pool_paths {
                lines.load_file(p, max_teams_per_file).map_err(|e| e.to_string())?;
            }
            for t in teams.iter().flatten() {
                lines.push_line(t);
            }
            let mut pool = Pool::new();
            for i in 0..lines.len() {
                pool.add_team(lines.get(i)).map_err(|e| format!("team {}: {}", i + 1, e.0))?;
            }
            pool.shrink_to_fit();
            Ok(pool)
        });
        let pool = pool.map_err(PyValueError::new_err)?;
        if pool.is_empty() {
            return Err(PyValueError::new_err("the set pool is empty"));
        }
        Ok(SetPool { pool: Arc::new(pool) })
    }

    fn __len__(&self) -> usize {
        self.pool.len()
    }

    /// Distinct sets stored.
    #[getter]
    fn distinct(&self) -> usize {
        self.pool.distinct()
    }
}

fn name(id: EffectId) -> Option<&'static str> {
    (id != EffectId::NONE).then(|| dex::effect(id).name)
}

fn type_name(t: TypeId) -> &'static str {
    dex::TYPE_NAMES[t.0 as usize - 1]
}

fn gender(g: Gender) -> &'static str {
    match g {
        Gender::Male => "M",
        Gender::Female => "F",
        _ => "",
    }
}

/// The public information of a tracker as a dict (both sides, every Pokemon seen so far).
fn tracker_dict<'py>(py: Python<'py>, t: &Tracker) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("turn", t.turn())?;
    let sides = PyList::empty(py);
    for side in [SideId(0), SideId(1)] {
        let s = PyDict::new(py);
        s.set_item("team_size", t.team_size(side))?;
        s.set_item("tera_used", t.tera_used(side))?;
        s.set_item("active", [t.active(side, 0), t.active(side, 1)])?;
        let mons = PyList::empty(py);
        for m in t.mons(side) {
            let md = PyDict::new(py);
            md.set_item("name", &m.name)?;
            md.set_item("species", &m.details_species)?;
            md.set_item("current_species", &m.current_species)?;
            md.set_item("level", m.level)?;
            md.set_item("gender", gender(m.gender))?;
            md.set_item("shiny", m.shiny)?;
            md.set_item("moves", m.moves.iter().filter_map(|&id| name(id)).collect::<Vec<_>>())?;
            md.set_item(
                "item",
                match m.item {
                    ItemKnowledge::Unknown => None,
                    ItemKnowledge::Held(id) => name(id),
                    ItemKnowledge::Lost => Some(""),
                },
            )?;
            md.set_item("starting_item", m.starting_item.and_then(name))?;
            md.set_item("ability", m.ability.and_then(name))?;
            md.set_item("starting_ability", m.starting_ability.and_then(name))?;
            md.set_item("tera_type", m.tera_type.map(type_name))?;
            md.set_item("terastallized", m.terastallized)?;
            md.set_item("hp", m.hp)?;
            md.set_item("max_hp", m.max_hp)?;
            md.set_item("status", &m.status)?;
            md.set_item("fainted", m.fainted)?;
            md.set_item("active", m.active)?;
            md.set_item("transformed", m.transformed)?;
            md.set_item("illusion", m.illusion)?;
            mons.append(md)?;
        }
        s.set_item("mons", mons)?;
        sides.append(s)?;
    }
    d.set_item("sides", sides)?;
    Ok(d)
}

/// Public information per Pokemon (both sides) from one player's player-view protocol lines: moves used,
/// item / ability / Tera type shown, HP, status. `RevealTracker()`, `feed(lines)`, `state()` -> dict.
/// Works from `Battle.drain_player_log` / `BatchEnv.observe` lines and from a Showdown server's stream.
#[pyclass(frozen, module = "numbrion")]
pub(crate) struct RevealTracker {
    t: Mutex<Tracker>,
}

#[pymethods]
impl RevealTracker {
    #[new]
    fn new() -> Self {
        RevealTracker { t: Mutex::new(Tracker::new()) }
    }

    fn feed(&self, lines: Vec<String>) {
        let mut t = lock(&self.t);
        for l in &lines {
            t.feed(l);
        }
    }

    fn state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        tracker_dict(py, &lock(&self.t))
    }
}

/// Fair-information determinized worlds for search, from one player's observations only.
///
/// `WorldBuilder(viewer, team, pool, names=None, seed=0, particles=8, segment_budget=2000, rebuild_budget=16)`:
/// `viewer` is the player's side (0/1); `team` its own packed team with genders resolved (`BatchEnv.team` /
/// `Battle.teams` of a battle built from explicit-gender pools); `pool` a `SetPool`. Feed, in order, every
/// player-view line (`push_lines`), and at each boundary where the player acts optionally its request
/// (`push_request`) and then its choice (`push_choice` text or `push_action` codes). `worlds(k, seed)` returns
/// `k` started `Battle`s at the current boundary, consistent with everything observed, reseeded from `seed`.
/// The real battle is never consulted, so the same code serves self-play and a ladder client.
#[pyclass(frozen, module = "numbrion")]
pub(crate) struct WorldBuilder {
    w: Mutex<Builder>,
}

#[pymethods]
impl WorldBuilder {
    #[new]
    #[pyo3(signature = (viewer, team, pool, names=None, seed=0, particles=8, segment_budget=2000, rebuild_budget=16))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        viewer: usize,
        team: &str,
        pool: &SetPool,
        names: Option<[String; 2]>,
        seed: u64,
        particles: usize,
        segment_budget: u32,
        rebuild_budget: usize,
    ) -> PyResult<Self> {
        if viewer >= 2 {
            return Err(PyValueError::new_err("viewer must be 0 or 1"));
        }
        if particles == 0 {
            return Err(PyValueError::new_err("particles must be positive"));
        }
        let team = TeamDef::unpack(team).map_err(|e| PyValueError::new_err(e.0))?;
        let names = names.unwrap_or(["Player 1".to_string(), "Player 2".to_string()]);
        let config = WorldConfig { particles, segment_budget, rebuild_budget, ..WorldConfig::default() };
        let w = Builder::new(SideId(viewer as u8), Arc::new(team), names, pool.pool.clone(), config, seed)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(WorldBuilder { w: Mutex::new(w) })
    }

    /// Player-view lines of the viewer, from the start of the battle, in order.
    fn push_lines(&self, lines: Vec<String>) {
        lock(&self.w).push_lines(lines.iter().map(String::as_str));
    }

    /// The viewer's request JSON at the boundary it is about to answer (optional, strengthens the check).
    fn push_request(&self, json: &str) {
        lock(&self.w).push_request(json);
    }

    /// The viewer's choice as Showdown text (`"move 1 +2, switch 3"`), before the lines it produced.
    fn push_choice(&self, text: &str) {
        lock(&self.w).push_choice(text);
    }

    /// The viewer's choice as two action codes.
    fn push_action(&self, a0: i64, a1: i64) -> PyResult<()> {
        let text = usize::try_from(a0)
            .ok()
            .zip(usize::try_from(a1).ok())
            .and_then(|(a, b)| pair_text(a, b))
            .ok_or_else(|| PyValueError::new_err(format!("bad action codes ({a0}, {a1})")))?;
        lock(&self.w).push_choice(&text);
        Ok(())
    }

    /// Bring the worlds up to date (also done by `worlds`). GIL released. Raises `RuntimeError` when no world
    /// reproduces the observations within the budgets.
    fn update(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| lock(&self.w).update()).map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// `k` worlds at the current boundary, drawn by likelihood weight, each reseeded from `seed`. `log` selects
    /// `TextLog` battles (player-view logs in `GridExecutor`) over `NoLog`. GIL released.
    #[pyo3(signature = (k, seed, log=false))]
    fn worlds(&self, py: Python<'_>, k: usize, seed: u64, log: bool) -> PyResult<Vec<Battle>> {
        let games = py
            .detach(|| -> Result<Vec<Game>, String> {
                let mut w = lock(&self.w);
                Ok(if log {
                    w.worlds::<TextLog>(k, seed).map_err(|e| e.to_string())?.into_iter().map(|b| Game::from_world(Sink::Text(b))).collect()
                } else {
                    w.worlds::<NoLog>(k, seed).map_err(|e| e.to_string())?.into_iter().map(|b| Game::from_world(Sink::No(b))).collect()
                })
            })
            .map_err(PyRuntimeError::new_err)?;
        Ok(games.into_iter().map(|g| Battle { g: Mutex::new(g) }).collect())
    }

    /// Worlds currently alive.
    #[getter]
    fn alive(&self) -> usize {
        lock(&self.w).alive()
    }

    /// `(weights, effective_sample_size)` of the current worlds.
    #[getter]
    fn weights(&self) -> (Vec<f64>, f64) {
        lock(&self.w).weights()
    }

    /// Cumulative counters: attempts (segment replays), segments, builds, deaths, copies, swaps.
    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let s = lock(&self.w).stats;
        let d = PyDict::new(py);
        d.set_item("attempts", s.attempts)?;
        d.set_item("segments", s.segments)?;
        d.set_item("builds", s.builds)?;
        d.set_item("deaths", s.deaths)?;
        d.set_item("copies", s.copies)?;
        d.set_item("swaps", s.swaps)?;
        Ok(d)
    }

    /// The public information the builder has tracked (`RevealTracker.state()` layout).
    fn public_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        tracker_dict(py, lock(&self.w).tracker())
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SetPool>()?;
    m.add_class::<RevealTracker>()?;
    m.add_class::<WorldBuilder>()?;
    Ok(())
}
