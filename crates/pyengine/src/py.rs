//! Python bindings (`numbrion._numbrion`). Thin wrappers over `game`, `mask` and `env`; all work that can
//! take time runs with the GIL released.

use crate::{
    action::{self, Act, N_ACTIONS},
    env::{BatchEnv as Env, Config, OnIllegal, RunStats, SamplerParams, StepOut},
    game::Game,
    mask::{Req, SideActions},
    pool::TeamPool,
};
use numpy::{AllowTypeChange, IntoPyArray, PyArray1, PyArrayLikeDyn, PyArrayMethods, PyUntypedArrayMethods};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

pyo3::create_exception!(
    _numbrion,
    ChoiceError,
    PyValueError,
    "The engine rejected a choice. `args[0]` is Showdown's error text (`[Invalid choice] ...`); `args[1]`, when \
     present, is the re-sent request JSON (hidden information revealed by the rejection)."
);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn choice_err(e: engine::sim::ChoiceError) -> PyErr {
    match e.resent_request_json {
        Some(r) => PyErr::new::<ChoiceError, _>((e.text, r)),
        None => PyErr::new::<ChoiceError, _>((e.text,)),
    }
}

fn check_side(side: usize) -> PyResult<()> {
    if side < 2 { Ok(()) } else { Err(PyValueError::new_err(format!("side must be 0 or 1, got {side}"))) }
}

fn bool_vec(mask: u64) -> Vec<bool> {
    (0..N_ACTIONS).map(|a| mask >> a & 1 == 1).collect()
}

fn bools<'py>(py: Python<'py>, v: &[u8]) -> Bound<'py, PyArray1<bool>> {
    PyArray1::from_vec(py, v.iter().map(|&b| b != 0).collect())
}

fn kind_name(r: Req) -> Option<&'static str> {
    match r {
        Req::None => None,
        Req::Move => Some("move"),
        Req::Switch => Some("switch"),
    }
}

fn decode_dict<'py>(py: Python<'py>, a: usize) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    match action::decode(a) {
        Some(Act::Move { slot, target, tera }) => {
            d.set_item("kind", "move")?;
            d.set_item("slot", slot)?;
            d.set_item("target", action::target_loc(target))?;
            d.set_item("tera", tera)?;
        }
        Some(Act::Switch(p)) => {
            d.set_item("kind", "switch")?;
            d.set_item("party", p)?;
        }
        Some(Act::Pass) => d.set_item("kind", "pass")?,
        None => return Err(PyValueError::new_err(format!("action code {a} out of range 0..{N_ACTIONS}"))),
    }
    Ok(d)
}

fn code(a: i64) -> PyResult<usize> {
    usize::try_from(a).ok().filter(|&a| a < N_ACTIONS).ok_or_else(|| PyValueError::new_err(format!("action code {a} out of range 0..{N_ACTIONS}")))
}

/// A single battle for debugging, scripted play and search.
#[pyclass(frozen, module = "numbrion")]
struct Battle {
    g: Mutex<Game>,
}

#[pymethods]
impl Battle {
    /// `Battle(seed, p1, p2, names=("Player 1", "Player 2"), log=False)`
    ///
    /// `seed`: four 16-bit PRNG words (Showdown's `a,b,c,d`); `p1`, `p2`: packed teams. With `log=True` the battle
    /// keeps its protocol log so `drain_log` / `drain_player_log` work; otherwise it runs with `NoLog`.
    #[new]
    #[pyo3(signature = (seed, p1, p2, names=None, log=false))]
    fn new(seed: [u16; 4], p1: &str, p2: &str, names: Option<[String; 2]>, log: bool) -> PyResult<Self> {
        let names = names.unwrap_or(["Player 1".to_string(), "Player 2".to_string()]);
        let g = Game::new(seed, Arc::from(p1), Arc::from(p2), names, log, true).map_err(PyValueError::new_err)?;
        Ok(Battle { g: Mutex::new(g) })
    }

    /// Run the battle to its first decision.
    fn start(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| lock(&self.g).start()).map_err(PyRuntimeError::new_err)
    }

    /// The side's current request as Showdown JSON text (`{"wait":true,...}` when waiting), or None after the end.
    fn request_json(&self, side: usize) -> PyResult<Option<String>> {
        check_side(side)?;
        Ok(lock(&self.g).request_json(side))
    }

    /// Submit a Showdown choice string (`"move 1 +2, switch 3"`). Raises `ChoiceError` with Showdown's error
    /// text on rejection. Returns None; when the last pending side has chosen, the battle advances to the next
    /// decision before this returns.
    fn choose(&self, py: Python<'_>, side: usize, text: &str) -> PyResult<()> {
        check_side(side)?;
        py.detach(|| lock(&self.g).choose(side, text)).map_err(choice_err)
    }

    /// Submit a choice given as two action codes (see `numbrion.describe_action`). Goes through the engine's
    /// typed path. Raises `ChoiceError` if the pair is not in the legal mask.
    fn choose_action(&self, py: Python<'_>, side: usize, a0: i64, a1: i64) -> PyResult<()> {
        check_side(side)?;
        let (a0, a1) = (code(a0)?, code(a1)?);
        py.detach(|| {
            let mut g = lock(&self.g);
            let acts = g.side_actions(side);
            match acts.choices(a0, a1, &g.party(side)) {
                Some(ch) => g.choose_typed(side, &ch),
                None => {
                    // Ask the engine on a throw-away copy what it makes of it, for Showdown's wording.
                    let mut probe = g.duplicate();
                    let ch = [probe.unchecked_choice(side, 0, a0), probe.unchecked_choice(side, 1, a1)];
                    match probe.choose_typed(side, &ch) {
                        Err(e) => Err(e),
                        Ok(()) => Err(engine::sim::ChoiceError {
                            text: format!(
                                "[Invalid choice] Action ({a0}, {a1}) = ({}, {}) is not a canonical legal action; see legal_mask",
                                action::describe(a0),
                                action::describe(a1)
                            ),
                            resent_request_json: None,
                        }),
                    }
                }
            }
        })
        .map_err(choice_err)
    }

    /// The Showdown choice text for a legal action pair, or None if the pair is not legal.
    fn action_text(&self, side: usize, a0: i64, a1: i64) -> PyResult<Option<String>> {
        check_side(side)?;
        Ok(lock(&self.g).side_actions(side).text(code(a0)?, code(a1)?))
    }

    /// The side has to submit a choice now.
    fn needs_action(&self, side: usize) -> PyResult<bool> {
        check_side(side)?;
        Ok(lock(&self.g).side_actions(side).needs_action())
    }

    /// `"move"`, `"switch"` or None (waiting for the other side, already answered, or over).
    fn request_kind(&self, side: usize) -> PyResult<Option<&'static str>> {
        check_side(side)?;
        Ok(kind_name(lock(&self.g).side_actions(side).req))
    }

    /// `(mask0, joint)`: `mask0[a0]` is True if slot 0 may take `a0`; `joint[a0, a1]` is True if the pair is a
    /// legal choice. `joint[a0]` is the slot-1 mask after slot 0 chose `a0`. All False for a side that needs no action.
    fn legal_mask<'py>(&self, py: Python<'py>, side: usize) -> PyResult<(Bound<'py, PyArray1<bool>>, Bound<'py, numpy::PyArray2<bool>>)> {
        check_side(side)?;
        let a = lock(&self.g).side_actions(side);
        let mut joint = vec![0u8; N_ACTIONS * N_ACTIONS];
        if a.needs_action() {
            a.write_joint(&mut joint);
        }
        let joint: Vec<bool> = joint.iter().map(|&b| b != 0).collect();
        let m0 = PyArray1::from_vec(py, bool_vec(if a.needs_action() { a.mask0() } else { 0 }));
        Ok((m0, PyArray1::from_vec(py, joint).reshape([N_ACTIONS, N_ACTIONS])?))
    }

    /// Slot-1 mask after slot 0 chose `a0`.
    fn legal_mask_slot1<'py>(&self, py: Python<'py>, side: usize, a0: i64) -> PyResult<Bound<'py, PyArray1<bool>>> {
        check_side(side)?;
        Ok(PyArray1::from_vec(py, bool_vec(lock(&self.g).side_actions(side).mask1(code(a0)?))))
    }

    /// Structured description of the side's options (debugging): request kind, per-slot individually legal
    /// action codes with names, and the joint budgets.
    fn legal_actions<'py>(&self, py: Python<'py>, side: usize) -> PyResult<Bound<'py, PyDict>> {
        check_side(side)?;
        let a: SideActions = lock(&self.g).side_actions(side);
        let d = PyDict::new(py);
        d.set_item("request", kind_name(a.req))?;
        let slots = PyList::empty(py);
        for k in 0..2 {
            let s = PyDict::new(py);
            let ls = &a.legal.slots[k];
            s.set_item("auto_pass", a.auto[k])?;
            s.set_item("revival", a.revival[k])?;
            s.set_item("forced_move", a.forced(k))?;
            s.set_item("can_pass", ls.can_pass)?;
            let codes: Vec<usize> = action::bits(a.slot_set[k]).collect();
            s.set_item("names", codes.iter().map(|&c| action::describe(c)).collect::<Vec<_>>())?;
            s.set_item("codes", codes)?;
            slots.append(s)?;
        }
        d.set_item("slots", slots)?;
        d.set_item("forced_switches", a.legal.constraints.forced_switches)?;
        d.set_item("forced_passes", a.legal.constraints.forced_passes)?;
        d.set_item("tera_budget", a.legal.constraints.terastallize_budget)?;
        Ok(d)
    }

    /// Battle turn counter.
    #[getter]
    fn turn(&self) -> u32 {
        lock(&self.g).turn()
    }

    #[getter]
    fn ended(&self) -> bool {
        lock(&self.g).ended()
    }

    /// 0 or 1 once a side has won; None while running and after a tie.
    #[getter]
    fn winner(&self) -> Option<usize> {
        lock(&self.g).outcome().and_then(|o| o.winner)
    }

    #[getter]
    fn tie(&self) -> bool {
        lock(&self.g).outcome().is_some_and(|o| o.tie)
    }

    /// Unfainted Pokemon per side at the end of the battle, None while running.
    #[getter]
    fn pokemon_left(&self) -> Option<(u32, u32)> {
        lock(&self.g).outcome().map(|o| (o.pokemon_left[0], o.pokemon_left[1]))
    }

    /// The current PRNG state as four 16-bit words (what `reseed` accepts).
    #[getter]
    fn prng_seed(&self) -> (u16, u16, u16, u16) {
        let s = lock(&self.g).prng_seed();
        (s[0], s[1], s[2], s[3])
    }

    #[getter]
    fn log_enabled(&self) -> bool {
        lock(&self.g).has_log()
    }

    /// Omniscient protocol lines (engine `battle.log` entries, `|split|pN` triples included, exactly as the
    /// Showdown fixtures store them) produced since the last call.
    fn drain_log(&self) -> PyResult<Vec<String>> {
        let mut g = lock(&self.g);
        if !g.has_log() {
            return Err(PyRuntimeError::new_err("battle was created with log=False"));
        }
        Ok(g.drain_omni())
    }

    /// What `side` has been shown since the last call: `|split|pN` triples resolved to the secret line for
    /// side N and the public line for the other side, empty lines dropped. The hidden-information-safe
    /// observation stream (same content as the per-player stream of Showdown's `BattleStream`).
    fn drain_player_log(&self, side: usize) -> PyResult<Vec<String>> {
        check_side(side)?;
        let mut g = lock(&self.g);
        if !g.has_log() {
            return Err(PyRuntimeError::new_err("battle was created with log=False"));
        }
        Ok(g.drain_player(side))
    }

    /// An independent copy at the current decision boundary. The PRNG state is copied too: the copy replays
    /// exactly what the original would do. Call `reseed` for search that must not know the real future
    /// randomness. Log buffers of the copy start empty. Cost is dominated by rebuilding the team definitions;
    /// for hot loops reuse a scratch battle with `copy_from`.
    fn clone(&self, py: Python<'_>) -> Battle {
        Battle { g: Mutex::new(py.detach(|| lock(&self.g).duplicate())) }
    }

    fn __copy__(&self, py: Python<'_>) -> Battle {
        self.clone(py)
    }

    fn __deepcopy__(&self, py: Python<'_>, _memo: &Bound<'_, PyAny>) -> Battle {
        self.clone(py)
    }

    /// Overwrite this battle's state with `other`'s (cheap, ~1 us). Both must come from the same constructor
    /// arguments (same seed, teams and names), e.g. a scratch `clone()` of the original.
    fn copy_from(&self, other: &Battle) -> PyResult<()> {
        if std::ptr::eq(self, other) {
            return Ok(());
        }
        let o = lock(&other.g);
        let mut g = lock(&self.g);
        if !g.same_origin(&o) {
            return Err(PyValueError::new_err("copy_from needs a battle built from the same seed, teams and names"));
        }
        g.copy_state_from(&o);
        Ok(())
    }

    /// Replace the PRNG state (four 16-bit words). The next draw is the first of the new stream.
    fn reseed(&self, seed: [u16; 4]) {
        lock(&self.g).reseed(seed);
    }

    /// Determinization: indices into the opponent's packed team (construction order, stable for the battle)
    /// of the Pokemon `viewer` has never seen and whose sets `replace_hidden_set` can swap now.
    fn hidden_team_indices(&self, viewer: usize) -> PyResult<Vec<usize>> {
        check_side(viewer)?;
        Ok(lock(&self.g).hidden_team_indices(viewer))
    }

    /// Give the opponent's never-revealed Pokemon at packed-team `index` the single packed set `packed_set`,
    /// in place, with no events, log lines or PRNG draws: the result equals constructing the battle with that set
    /// and replaying the same choices (docs/design/TRAINING-API.md). Raises ValueError, battle unchanged, when
    /// the Pokemon was revealed or the swap is unsupported. Call `reseed` before searching the world.
    fn replace_hidden_set(&self, viewer: usize, index: usize, packed_set: &str) -> PyResult<()> {
        check_side(viewer)?;
        lock(&self.g).replace_hidden_set(viewer, index, packed_set).map_err(PyValueError::new_err)
    }

    #[getter]
    fn names(&self) -> (String, String) {
        let g = lock(&self.g);
        (g.names()[0].clone(), g.names()[1].clone())
    }

    #[getter]
    fn teams(&self) -> (String, String) {
        let g = lock(&self.g);
        (g.packed_teams()[0].to_string(), g.packed_teams()[1].to_string())
    }
}

fn out_to_dict<'py>(py: Python<'py>, o: StepOut) -> PyResult<Bound<'py, PyDict>> {
    let n = o.n;
    let a = N_ACTIONS;
    let d = PyDict::new(py);
    d.set_item("needs_action", bools(py, &o.needs_action).reshape([n, 2])?)?;
    d.set_item("request_kind", o.request_kind.into_pyarray(py).reshape([n, 2])?)?;
    d.set_item("mask0", bools(py, &o.mask0).reshape([n, 2, a])?)?;
    d.set_item("mask1_any", bools(py, &o.mask1_any).reshape([n, 2, a])?)?;
    if let Some(j) = &o.joint {
        d.set_item("joint", bools(py, j).reshape([n, 2, a, a])?)?;
    }
    d.set_item("reward", o.reward.into_pyarray(py).reshape([n, 2])?)?;
    d.set_item("done", bools(py, &o.done))?;
    d.set_item("winner", o.winner.into_pyarray(py))?;
    d.set_item("turn", o.turn.into_pyarray(py))?;
    d.set_item("final_turns", o.final_turns.into_pyarray(py))?;
    d.set_item("battle_id", o.battle_id.into_pyarray(py))?;
    d.set_item("illegal", bools(py, &o.illegal).reshape([n, 2])?)?;
    d.set_item("decisions", o.decisions)?;
    Ok(d)
}

fn env_err(e: String) -> PyErr {
    if e.contains("is not legal") { PyValueError::new_err(e) } else { PyRuntimeError::new_err(e) }
}

fn stats_dict<'py>(py: Python<'py>, s: RunStats, seconds: f64) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("battles", s.battles)?;
    d.set_item("decisions", s.decisions)?;
    d.set_item("boundaries", s.boundaries)?;
    d.set_item("turns", s.turns)?;
    d.set_item("illegal", s.illegal)?;
    d.set_item("seconds", seconds)?;
    Ok(d)
}

/// N battles stepped in parallel with auto-reset.
///
/// `BatchEnv(n_envs, team_pool_paths, seed=0, threads=0, log=False, joint_mask=False, on_illegal="raise",
/// max_teams_per_file=None, teams=None)`
#[pyclass(frozen, module = "numbrion")]
struct BatchEnv {
    env: Mutex<Env>,
    n_envs: usize,
    threads: usize,
    pool_size: usize,
}

#[pymethods]
impl BatchEnv {
    #[new]
    #[pyo3(signature = (n_envs, team_pool_paths=Vec::new(), seed=0, threads=0, log=false, joint_mask=false,
                        on_illegal="raise", max_teams_per_file=None, teams=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        n_envs: usize,
        team_pool_paths: Vec<PathBuf>,
        seed: u64,
        threads: usize,
        log: bool,
        joint_mask: bool,
        on_illegal: &str,
        max_teams_per_file: Option<usize>,
        teams: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let on_illegal = match on_illegal {
            "raise" => OnIllegal::Raise,
            "default" => OnIllegal::Default,
            other => return Err(PyValueError::new_err(format!("on_illegal must be 'raise' or 'default', got {other:?}"))),
        };
        let threads = if threads == 0 { std::thread::available_parallelism().map_or(1, |n| n.get()) } else { threads };
        let cfg = Config { n_envs, seed, threads, log, joint_mask, on_illegal };
        let env = py.detach(move || -> Result<(Env, usize), String> {
            let mut pool = TeamPool::default();
            for p in &team_pool_paths {
                pool.load_file(p, max_teams_per_file).map_err(|e| e.to_string())?;
            }
            for t in teams.iter().flatten() {
                pool.push_line(t);
            }
            let size = pool.len();
            Ok((Env::new(cfg, Arc::new(pool))?, size))
        });
        let (env, pool_size) = env.map_err(PyValueError::new_err)?;
        Ok(BatchEnv { env: Mutex::new(env), n_envs, threads, pool_size })
    }

    #[getter]
    fn n_envs(&self) -> usize {
        self.n_envs
    }

    #[getter]
    fn threads(&self) -> usize {
        self.threads
    }

    /// Number of packed teams in the sampling pool.
    #[getter]
    fn pool_size(&self) -> usize {
        self.pool_size
    }

    /// Start a fresh battle in every environment; returns the same dict as `step`.
    fn reset<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let o = py.detach(|| lock(&self.env).reset()).map_err(env_err)?;
        out_to_dict(py, o)
    }

    /// The decision information of the current boundary (what the last `step`/`reset` returned), without stepping.
    fn current<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let o = lock(&self.env).current();
        out_to_dict(py, o)
    }

    /// `step(actions)`: `actions` is an int32-compatible array `[n_envs, 2 sides, 2 slots]` of action codes.
    /// Entries of sides that need no action are ignored. Advances every environment to its next decision and
    /// auto-resets finished battles. Returns a dict of numpy arrays, see docs/design/TRAINING-API.md.
    fn step<'py>(&self, py: Python<'py>, actions: PyArrayLikeDyn<'py, i32, AllowTypeChange>) -> PyResult<Bound<'py, PyDict>> {
        let shape = actions.shape().to_vec();
        if shape != [self.n_envs, 2, 2] {
            return Err(PyValueError::new_err(format!("actions must have shape ({}, 2, 2), got {shape:?}", self.n_envs)));
        }
        let flat: Vec<i32> = actions.as_array().iter().copied().collect();
        let o = py.detach(|| lock(&self.env).step(&flat)).map_err(env_err)?;
        out_to_dict(py, o)
    }

    /// `[n_envs, 2, N_ACTIONS]` mask of slot 1 after slot 0 chose `a0[env, side]` (int array `[n_envs, 2]`).
    fn mask_slot1<'py>(&self, py: Python<'py>, a0: PyArrayLikeDyn<'py, i32, AllowTypeChange>) -> PyResult<Bound<'py, numpy::PyArray3<bool>>> {
        let shape = a0.shape().to_vec();
        if shape != [self.n_envs, 2] {
            return Err(PyValueError::new_err(format!("a0 must have shape ({}, 2), got {shape:?}", self.n_envs)));
        }
        let flat: Vec<i32> = a0.as_array().iter().copied().collect();
        let m = py.detach(|| lock(&self.env).mask_slot1(&flat)).map_err(env_err)?;
        Ok(bools(py, &m).reshape([self.n_envs, 2, N_ACTIONS])?)
    }

    /// Random legal actions `[n_envs, 2, 2]` int32 (-1 for sides that need no action), drawn in Rust. A side
    /// switches with `switch_prob` when it can, picks a uniform move, then a uniform target, and terastallizes
    /// with `tera_prob` when offered (the Showdown fixture generator's picker).
    #[pyo3(signature = (switch_prob=0.1, tera_prob=0.15))]
    fn random_actions<'py>(&self, py: Python<'py>, switch_prob: f64, tera_prob: f64) -> PyResult<Bound<'py, numpy::PyArray3<i32>>> {
        let p = SamplerParams { switch_prob, tera_prob };
        let v = py.detach(|| lock(&self.env).random_actions(&p));
        Ok(v.into_pyarray(py).reshape([self.n_envs, 2, 2])?)
    }

    /// What `side` of environment `env` is allowed to see: `request` (its own request JSON), `log` (new
    /// player-view protocol lines since the last call; only with `log=True`), `prev_log` (unread lines of the
    /// battle that finished at the last auto-reset), plus `turn` and `battle_id`.
    fn observe<'py>(&self, py: Python<'py>, env: usize, side: usize) -> PyResult<Bound<'py, PyDict>> {
        let mut e = lock(&self.env);
        let log_enabled = e.config().log;
        let o = e.observe(env, side).map_err(PyValueError::new_err)?;
        let d = PyDict::new(py);
        d.set_item("request", o.request)?;
        d.set_item("log", o.log)?;
        d.set_item("prev_log", o.prev_log)?;
        d.set_item("turn", o.turn)?;
        d.set_item("battle_id", o.battle_id)?;
        d.set_item("log_enabled", log_enabled)?;
        Ok(d)
    }

    /// A `numbrion.Battle` copy of environment `env` at its current decision boundary (for search).
    fn clone_battle(&self, py: Python<'_>, env: usize) -> PyResult<Battle> {
        let g = py.detach(|| lock(&self.env).clone_game(env)).map_err(PyValueError::new_err)?;
        Ok(Battle { g: Mutex::new(g) })
    }

    /// Cumulative counters since construction: battles, decisions (side choices), boundaries, turns, illegal.
    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        stats_dict(py, lock(&self.env).stats(), 0.0)
    }

    /// Benchmark driver that never leaves Rust: every environment plays random legal self-play until it has
    /// finished `battles_per_env` more battles. Returns counters and the elapsed wall-clock seconds.
    #[pyo3(signature = (battles_per_env, switch_prob=0.1, tera_prob=0.15))]
    fn run_random<'py>(&self, py: Python<'py>, battles_per_env: u64, switch_prob: f64, tera_prob: f64) -> PyResult<Bound<'py, PyDict>> {
        let p = SamplerParams { switch_prob, tera_prob };
        let (s, secs) = py
            .detach(|| {
                let t = Instant::now();
                lock(&self.env).run_random(battles_per_env, &p).map(|s| (s, t.elapsed().as_secs_f64()))
            })
            .map_err(env_err)?;
        stats_dict(py, s, secs)
    }
}

#[pyfunction]
fn describe_action(a: i64) -> PyResult<String> {
    Ok(action::describe(code(a)?))
}

#[pyfunction]
fn decode_action<'py>(py: Python<'py>, a: i64) -> PyResult<Bound<'py, PyDict>> {
    decode_dict(py, code(a)?)
}

#[pyfunction]
fn move_action(slot: u8, target: i8, tera: bool) -> PyResult<usize> {
    if slot >= 4 || !(-2..=2).contains(&target) {
        return Err(PyValueError::new_err("slot must be 0..4 and target -2..=2 (0 = no target)"));
    }
    Ok(action::move_action(slot, (target + 2) as u8, tera))
}

#[pyfunction]
fn switch_action(party: u8) -> PyResult<usize> {
    if party >= 6 {
        return Err(PyValueError::new_err("party index must be 0..6"));
    }
    Ok(action::switch_action(party))
}

#[pymodule]
fn _numbrion(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Battle>()?;
    m.add_class::<BatchEnv>()?;
    m.add("ChoiceError", m.py().get_type::<ChoiceError>())?;
    m.add_function(wrap_pyfunction!(describe_action, m)?)?;
    m.add_function(wrap_pyfunction!(decode_action, m)?)?;
    m.add_function(wrap_pyfunction!(move_action, m)?)?;
    m.add_function(wrap_pyfunction!(switch_action, m)?)?;
    m.add("N_ACTIONS", N_ACTIONS)?;
    m.add("N_MOVE_ACTIONS", action::N_MOVE_ACTIONS)?;
    m.add("SWITCH_BASE", action::SWITCH_BASE)?;
    m.add("PASS", action::PASS)?;
    m.add("FORCED_MOVE", action::FORCED_MOVE)?;
    m.add("TARGET_AUTO", action::TARGET_AUTO)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
