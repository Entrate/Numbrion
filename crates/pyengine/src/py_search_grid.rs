//! Python bindings of the fused search-leaf encoder: `TrackerState` (an observation tracker snapshot) and
//! `SearchGrid` (`search_grid.rs`). Work runs with the GIL released. See docs/design/TRAINING-API.md,
//! "Fused leaf encoding".

use crate::{
    py::{Battle, GridArgs, grid_args, grid_err, lock, put_grid_arrays},
    search_grid::{LeafOptions, PARTNER_IDS_PER_ROW, SearchGrid as Grid, StateInput},
};
use encoder::{
    Dex,
    encode::GUARD_PER_ROW,
    layout::{N_ACTION_FEATURES, N_CODES, N_FIELD_FLOATS, N_IDS, N_POKEMON_FLOATS, N_TOKEN_FEATURES, N_TOKENS},
    tracker::Tracker,
};
use numpy::{AllowTypeChange, IntoPyArray, PyArray1, PyArrayLikeDyn, PyArrayMethods};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyDict,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// An immutable snapshot of one observation tracker: everything `ObsEncoder.encode_row` reads for a row besides
/// the new lines and the request (the public battle state rebuilt from that side's player-view lines).
///
/// `TrackerState(side)` is an empty tracker for side 0 (p1) or 1 (p2); `ObsEncoder.snapshot(row)` copies a row's.
/// `feed(lines)` returns a new state with the lines applied. `side`, `turn` and `battle_id` describe it.
#[pyclass(frozen, module = "numbrion")]
pub(crate) struct TrackerState {
    pub(crate) tracker: Arc<Tracker>,
}

#[pymethods]
impl TrackerState {
    #[new]
    fn new(side: usize) -> PyResult<Self> {
        if side > 1 {
            return Err(PyValueError::new_err(format!("side must be 0 or 1, got {side}")));
        }
        Ok(TrackerState { tracker: Arc::new(Tracker::new(side)) })
    }

    #[getter]
    fn side(&self) -> usize {
        self.tracker.side
    }

    #[getter]
    fn turn(&self) -> i64 {
        self.tracker.turn
    }

    /// The battle id of the observations this row was last fed (-1 for a fresh tracker).
    #[getter]
    fn battle_id(&self) -> i64 {
        self.tracker.battle_id
    }

    /// A new state: a copy of this one fed with `lines` (player-view protocol lines).
    fn feed(&self, py: Python<'_>, lines: Vec<String>) -> TrackerState {
        let tracker = py.detach(|| {
            let mut t = (*self.tracker).clone();
            t.feed(&lines);
            t
        });
        TrackerState { tracker: Arc::new(tracker) }
    }

    fn __repr__(&self) -> String {
        let t = &self.tracker;
        format!("TrackerState(side={}, turn={}, battle_id={}, pokemon={})", t.side, t.turn, t.battle_id, t.mons.len())
    }
}

impl TrackerState {
    pub(crate) fn of(tracker: Tracker) -> Self {
        TrackerState { tracker: Arc::new(tracker) }
    }
}

/// `trackers`: one `(side-0 state, side-1 state)` pair, or a list with one pair per root.
fn tracker_pairs(trackers: &Bound<'_, PyAny>, n_roots: usize) -> PyResult<Vec<[Arc<Tracker>; 2]>> {
    let shape_err = || PyValueError::new_err("trackers must be a (TrackerState, TrackerState) pair or a list of such pairs");
    let items: Vec<Bound<'_, PyAny>> = trackers.extract().map_err(|_| shape_err())?;
    let pair = |items: &[Bound<'_, PyAny>]| -> PyResult<[Arc<Tracker>; 2]> {
        if items.len() != 2 {
            return Err(shape_err());
        }
        let get = |x: &Bound<'_, PyAny>| x.cast::<TrackerState>().map(|s| s.get().tracker.clone()).map_err(|_| shape_err());
        Ok([get(&items[0])?, get(&items[1])?])
    };
    let pairs = if items.len() == 2 && items[0].cast::<TrackerState>().is_ok() {
        vec![pair(&items)?]
    } else {
        items.iter().map(|p| pair(&p.extract::<Vec<Bound<'_, PyAny>>>().map_err(|_| shape_err())?)).collect::<PyResult<_>>()?
    };
    if pairs.len() != 1 && pairs.len() != n_roots {
        return Err(PyValueError::new_err(format!("trackers must be one pair or one pair per root ({n_roots}), got {}", pairs.len())));
    }
    Ok(pairs)
}

/// Search grid cells (`GridExecutor`) with each leaf encoded for the value network on the worker threads.
///
/// `SearchGrid(dex_path, threads=0)`: a persistent pool of Rust threads, each with a reusable worker battle, a
/// scratch tracker and a static-feature cache (`threads=0`: all hardware threads). See
/// docs/design/TRAINING-API.md, "Fused leaf encoding".
#[pyclass(frozen, module = "numbrion")]
struct SearchGrid {
    grid: Mutex<Grid>,
    threads: usize,
}

fn floats3<'py>(py: Python<'py>, v: Vec<f32>, shape: [usize; 3]) -> PyResult<Bound<'py, PyAny>> {
    Ok(v.into_pyarray(py).reshape(shape)?.into_any())
}

fn ids3<'py>(py: Python<'py>, v: Vec<i64>, shape: [usize; 3]) -> PyResult<Bound<'py, PyAny>> {
    Ok(v.into_pyarray(py).reshape(shape)?.into_any())
}

fn guard_array<'py>(py: Python<'py>, v: Vec<bool>, n: usize) -> PyResult<Bound<'py, PyAny>> {
    debug_assert_eq!(v.len(), n * GUARD_PER_ROW);
    Ok(PyArray1::from_vec(py, v).reshape([n, 2, 4, 2])?.into_any())
}

#[pymethods]
impl SearchGrid {
    #[new]
    #[pyo3(signature = (dex_path, threads=0))]
    fn new(py: Python<'_>, dex_path: PathBuf, threads: usize) -> PyResult<Self> {
        let threads = if threads == 0 { std::thread::available_parallelism().map_or(1, |n| n.get()) } else { threads };
        let dex = py.detach(|| Dex::shared(&dex_path)).map_err(PyValueError::new_err)?;
        let grid = Grid::new(dex, threads).map_err(PyRuntimeError::new_err)?;
        Ok(SearchGrid { grid: Mutex::new(grid), threads })
    }

    #[getter]
    fn threads(&self) -> usize {
        self.threads
    }

    /// `run(roots, actions, trackers, eval_side, world=None, seeds=None, stop="boundary", masks=False,
    /// action_features=False, guard=False)` -> dict of fresh arrays, one row per cell in input order.
    ///
    /// The cells are those of `GridExecutor.run` (same `roots`, `actions`, `world`, `seeds`, `stop` and the same
    /// validation; roots need `log=True`). `trackers` are the observation trackers of sides 0 and 1 at the root:
    /// one `(TrackerState, TrackerState)` pair for all roots, or a list with one pair per root. `eval_side` is the
    /// searching player's side.
    ///
    /// Per cell: `eval_side[i]` is `eval_side` if that side must act at the leaf, else the other side if it must,
    /// else -1 (the battle ended; every array row is zero). The eval side's observation (its tracker + its
    /// player-view lines from the root to the leaf + its leaf request) is in `ids`, `floats`, `field`,
    /// `token_features` (and `action_features`, `guard` when asked); the other side's own tokens 0-5 (the value
    /// head's partner input) in `partner_ids`, `partner_floats`. Also `ended`, `winner`, `turn`, `steps`,
    /// `needs_action`, `request_kind` (and `mask0`, `mask1_any` with `masks=True`) as `GridExecutor.run` returns.
    #[pyo3(signature = (roots, actions, trackers, eval_side, world=None, seeds=None, stop="boundary", masks=false,
                        action_features=false, guard=false))]
    #[allow(clippy::too_many_arguments)]
    fn run<'py>(
        &self,
        py: Python<'py>,
        roots: &Bound<'py, PyAny>,
        actions: PyArrayLikeDyn<'py, i32, AllowTypeChange>,
        trackers: &Bound<'py, PyAny>,
        eval_side: usize,
        world: Option<PyArrayLikeDyn<'py, i64, AllowTypeChange>>,
        seeds: Option<PyArrayLikeDyn<'py, i64, AllowTypeChange>>,
        stop: &str,
        masks: bool,
        action_features: bool,
        guard: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let GridArgs { roots, n, actions, world, seeds, stop } = grid_args(roots, actions, world, seeds, stop)?;
        let pairs = tracker_pairs(trackers, roots.len())?;
        let opts = LeafOptions { stop, masks, action_features, guard };
        let refs: Vec<&Battle> = roots.iter().map(|b| b.get()).collect();
        let out = py
            .detach(|| {
                let pairs: Vec<[&Tracker; 2]> = pairs.iter().map(|[a, b]| [&**a, &**b]).collect();
                let mut grid = lock(&self.grid);
                for (k, r) in refs.iter().enumerate() {
                    grid.stage_root(k, &lock(&r.g))?;
                }
                grid.run_staged(refs.len(), &world, &actions, &seeds, &pairs, eval_side, &opts)
            })
            .map_err(grid_err)?;
        let d = PyDict::new(py);
        put_grid_arrays(&d, out.grid)?;
        d.set_item("eval_side", out.eval_side.into_pyarray(py))?;
        d.set_item("ids", ids3(py, out.ids, [n, N_TOKENS, N_IDS])?)?;
        d.set_item("floats", floats3(py, out.floats, [n, N_TOKENS, N_POKEMON_FLOATS])?)?;
        d.set_item("field", out.field.into_pyarray(py).reshape([n, N_FIELD_FLOATS])?)?;
        d.set_item("token_features", floats3(py, out.token_features, [n, N_TOKENS, N_TOKEN_FEATURES])?)?;
        debug_assert_eq!(out.partner_ids.len(), n * PARTNER_IDS_PER_ROW);
        d.set_item("partner_ids", ids3(py, out.partner_ids, [n, 6, N_IDS])?)?;
        d.set_item("partner_floats", floats3(py, out.partner_floats, [n, 6, N_POKEMON_FLOATS])?)?;
        if let Some(a) = out.action_features {
            d.set_item("action_features", a.into_pyarray(py).reshape([n, 2, N_CODES, N_ACTION_FEATURES])?)?;
        }
        if let Some(g) = out.guard {
            d.set_item("guard", guard_array(py, g, n)?)?;
        }
        Ok(d)
    }

    /// `encode_states(states, requests, lines=None, action_features=True, guard=True)` -> dict of fresh arrays.
    ///
    /// Encodes hypothetical observations without touching any encoder: row `i` is a copy of `states[i]` (a
    /// `TrackerState`) fed with `lines[i]` (a list of player-view lines; None: no lines), encoded with
    /// `requests[i]` (request JSON, or None / "" for none), plus damage features: what `ObsEncoder.encode_batch`
    /// writes for a row with that history, bit for bit. Keys `ids` [n, 12, N_IDS], `floats`, `field`,
    /// `token_features`, and `action_features` / `guard` unless disabled. Runs on the grid's threads.
    #[pyo3(signature = (states, requests, lines=None, action_features=true, guard=true))]
    fn encode_states<'py>(
        &self,
        py: Python<'py>,
        states: Vec<Bound<'py, TrackerState>>,
        requests: Vec<Option<String>>,
        lines: Option<Vec<Option<Vec<String>>>>,
        action_features: bool,
        guard: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let n = states.len();
        if requests.len() != n {
            return Err(PyValueError::new_err(format!("requests must have one entry per state ({n}), got {}", requests.len())));
        }
        let lines = lines.unwrap_or_default();
        if !lines.is_empty() && lines.len() != n {
            return Err(PyValueError::new_err(format!("lines must have one entry per state ({n}), got {}", lines.len())));
        }
        let trackers: Vec<Arc<Tracker>> = states.iter().map(|s| s.get().tracker.clone()).collect();
        let out = py
            .detach(|| {
                let empty: Vec<String> = Vec::new();
                let inputs: Vec<StateInput<'_, String>> = (0..n)
                    .map(|i| StateInput {
                        tracker: &trackers[i],
                        request: requests[i].as_deref(),
                        lines: lines.get(i).and_then(|l| l.as_deref()).unwrap_or(&empty),
                    })
                    .collect();
                lock(&self.grid).encode_states(&inputs, action_features, guard)
            })
            .map_err(PyValueError::new_err)?;
        let d = PyDict::new(py);
        d.set_item("ids", ids3(py, out.ids, [n, N_TOKENS, N_IDS])?)?;
        d.set_item("floats", floats3(py, out.floats, [n, N_TOKENS, N_POKEMON_FLOATS])?)?;
        d.set_item("field", out.field.into_pyarray(py).reshape([n, N_FIELD_FLOATS])?)?;
        d.set_item("token_features", floats3(py, out.token_features, [n, N_TOKENS, N_TOKEN_FEATURES])?)?;
        if let Some(a) = out.action_features {
            d.set_item("action_features", a.into_pyarray(py).reshape([n, 2, N_CODES, N_ACTION_FEATURES])?)?;
        }
        if let Some(g) = out.guard {
            d.set_item("guard", guard_array(py, g, n)?)?;
        }
        Ok(d)
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<TrackerState>()?;
    m.add_class::<SearchGrid>()?;
    Ok(())
}
