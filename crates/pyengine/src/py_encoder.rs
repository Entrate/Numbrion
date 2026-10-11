//! NumPy observation encoding and scripted play. Array borrows stay alive while Rust runs without the GIL.

use crate::py::{BatchEnv, lock};
use encoder::{Dex, encode::{Encoder, Observation, Outputs}, layout::*, smart::{ObsRows, SmartConfig}};
use numpy::{Element, PyArray1, PyArrayDyn, PyArrayMethods, PyReadonlyArrayDyn, PyReadwriteArrayDyn,
            PyUntypedArrayMethods};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use std::{path::PathBuf, sync::Mutex};

type Logs = Vec<(Vec<String>, Vec<String>)>;

fn array<'py, T: Element>(value: &Bound<'py, PyAny>, name: &str, dtype: &str, shape: &[usize])
    -> PyResult<Bound<'py, PyArrayDyn<T>>>
{
    let a = value.cast::<PyArrayDyn<T>>()
        .map_err(|_| PyValueError::new_err(format!("{name} must be a numpy array with dtype {dtype}")))?;
    if a.shape() != shape {
        return Err(PyValueError::new_err(format!("{name} must have shape {shape:?}, got {:?}", a.shape())));
    }
    if !a.is_c_contiguous() {
        return Err(PyValueError::new_err(format!("{name} must be C-contiguous")));
    }
    Ok(a.clone())
}

fn writable<'py, T: Element>(value: &Bound<'py, PyAny>, name: &str, dtype: &str, shape: &[usize])
    -> PyResult<PyReadwriteArrayDyn<'py, T>>
{
    array(value, name, dtype, shape)?.try_readwrite()
        .map_err(|e| PyValueError::new_err(format!("{name} must be writable and exclusively borrowed: {e}")))
}

fn readable<'py, T: Element>(value: &Bound<'py, PyAny>, name: &str, dtype: &str, shape: &[usize])
    -> PyResult<PyReadonlyArrayDyn<'py, T>>
{
    array(value, name, dtype, shape)?.try_readonly()
        .map_err(|e| PyValueError::new_err(format!("{name} cannot be borrowed: {e}")))
}

// All shapes are checked before the encoder can change its trackers or drain environment logs.
#[allow(clippy::too_many_arguments)]
fn with_outputs<R>(rows: usize, ids: &Bound<'_, PyAny>, floats: &Bound<'_, PyAny>, field: &Bound<'_, PyAny>,
    foe_match: &Bound<'_, PyAny>, fainted: &Bound<'_, PyAny>, guard: &Bound<'_, PyAny>,
    action_features: &Bound<'_, PyAny>, token_features: &Bound<'_, PyAny>,
    f: impl FnOnce(&mut Outputs<'_>) -> PyResult<R>) -> PyResult<R>
{
    let mut ids = writable::<i64>(ids, "ids", "int64", &[rows, N_TOKENS, N_IDS])?;
    let mut floats = writable::<f32>(floats, "floats", "float32", &[rows, N_TOKENS, N_POKEMON_FLOATS])?;
    let mut field = writable::<f32>(field, "field", "float32", &[rows, N_FIELD_FLOATS])?;
    let mut foe_match = writable::<i64>(foe_match, "foe_match", "int64", &[rows, 6])?;
    let mut fainted = writable::<f32>(fainted, "fainted", "float32", &[rows, 2])?;
    let mut guard = writable::<bool>(guard, "guard", "bool", &[rows, 2, 4, 2])?;
    let mut action_features = writable::<f32>(action_features, "action_features", "float32", &[rows, 2, N_CODES, N_ACTION_FEATURES])?;
    let mut token_features = writable::<f32>(token_features, "token_features", "float32", &[rows, N_TOKENS, N_TOKEN_FEATURES])?;
    let mut out = Outputs {
        ids: ids.as_slice_mut()?, floats: floats.as_slice_mut()?, field: field.as_slice_mut()?,
        foe_match: foe_match.as_slice_mut()?, fainted: fainted.as_slice_mut()?, guard: guard.as_slice_mut()?,
        action_features: action_features.as_slice_mut()?, token_features: token_features.as_slice_mut()?,
    };
    out.rows().map_err(PyValueError::new_err)?;
    f(&mut out)
}

struct OwnedObservation {
    battle_id: i64,
    request: Option<String>,
    log: Vec<String>,
}

impl OwnedObservation {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let d = value.cast::<PyDict>().map_err(|_| PyValueError::new_err("observation must be a dict"))?;
        let get = |key: &str| d.get_item(key)?.ok_or_else(|| PyValueError::new_err(format!("observation is missing {key}")));
        Ok(Self {
            battle_id: get("battle_id")?.extract().map_err(|_| PyValueError::new_err("battle_id must be an int"))?,
            request: get("request")?.extract().map_err(|_| PyValueError::new_err("request must be a str or None"))?,
            log: get("log")?.extract().map_err(|_| PyValueError::new_err("log must be a list of str"))?,
        })
    }

    fn view(&self) -> Observation<'_, String> {
        Observation { battle_id: self.battle_id, request: self.request.as_deref(), log: &self.log }
    }
}

/// Encodes into caller-owned NumPy buffers. Do not mutate these arrays concurrently with encoding.
#[pyclass(frozen, module = "numbrion")]
struct ObsEncoder {
    encoder: Mutex<Encoder>,
    rows: usize,
}

#[pymethods]
impl ObsEncoder {
    #[new]
    fn new(py: Python<'_>, dex_path: PathBuf, rows: isize) -> PyResult<Self> {
        let rows = usize::try_from(rows).map_err(|_| PyValueError::new_err("rows must be nonnegative"))?;
        let encoder = py.detach(|| {
            let dex = Dex::shared(&dex_path)?;
            Ok::<_, String>(Encoder::new(dex, rows))
        }).map_err(PyValueError::new_err)?;
        Ok(Self { encoder: Mutex::new(encoder), rows })
    }

    #[getter]
    fn rows(&self) -> usize { self.rows }

    #[allow(clippy::too_many_arguments)]
    fn encode_batch(&self, py: Python<'_>, observations: Vec<Bound<'_, PyAny>>,
        ids: &Bound<'_, PyAny>, floats: &Bound<'_, PyAny>, field: &Bound<'_, PyAny>,
        foe_match: &Bound<'_, PyAny>, fainted: &Bound<'_, PyAny>, guard: &Bound<'_, PyAny>,
        action_features: &Bound<'_, PyAny>, token_features: &Bound<'_, PyAny>) -> PyResult<()>
    {
        if observations.len() != self.rows {
            return Err(PyValueError::new_err(format!("expected {} observations, got {}", self.rows, observations.len())));
        }
        let owned: Vec<_> = observations.iter().map(OwnedObservation::extract).collect::<PyResult<_>>()?;
        with_outputs(self.rows, ids, floats, field, foe_match, fainted, guard, action_features, token_features, |out| {
            py.detach(|| {
                let views: Vec<_> = owned.iter().map(OwnedObservation::view).collect();
                lock(&self.encoder).encode_batch(&views, out)
            }).map_err(PyValueError::new_err)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_row(&self, py: Python<'_>, row: isize, observation: &Bound<'_, PyAny>,
        ids: &Bound<'_, PyAny>, floats: &Bound<'_, PyAny>, field: &Bound<'_, PyAny>,
        foe_match: &Bound<'_, PyAny>, fainted: &Bound<'_, PyAny>, guard: &Bound<'_, PyAny>,
        action_features: &Bound<'_, PyAny>, token_features: &Bound<'_, PyAny>) -> PyResult<()>
    {
        let row = usize::try_from(row).ok().filter(|&r| r < self.rows)
            .ok_or_else(|| PyValueError::new_err(format!("row must be in 0..{}, got {row}", self.rows)))?;
        let owned = OwnedObservation::extract(observation)?;
        with_outputs(self.rows, ids, floats, field, foe_match, fainted, guard, action_features, token_features, |out| {
            py.detach(|| lock(&self.encoder).encode_row(row, &owned.view(), out)).map_err(PyValueError::new_err)
        })
    }

    /// Drain each player view once; partner matching and damage features run after all rows are encoded.
    #[pyo3(signature = (env, ids, floats, field, foe_match, fainted, guard, action_features, token_features, keep_logs=false))]
    #[allow(clippy::too_many_arguments)]
    fn encode_env(&self, py: Python<'_>, env: &BatchEnv,
        ids: &Bound<'_, PyAny>, floats: &Bound<'_, PyAny>, field: &Bound<'_, PyAny>,
        foe_match: &Bound<'_, PyAny>, fainted: &Bound<'_, PyAny>, guard: &Bound<'_, PyAny>,
        action_features: &Bound<'_, PyAny>, token_features: &Bound<'_, PyAny>, keep_logs: bool) -> PyResult<Option<Logs>>
    {
        with_outputs(self.rows, ids, floats, field, foe_match, fainted, guard, action_features, token_features, |out| {
            py.detach(|| {
                let mut encoder = lock(&self.encoder);
                let mut env = lock(&env.env);
                if 2 * env.n_envs() != self.rows {
                    return Err(format!("encoder has {} rows, expected 2 * env.n_envs = {}", self.rows, 2 * env.n_envs()));
                }
                let mut logs = keep_logs.then(|| Vec::with_capacity(self.rows));
                for e in 0..env.n_envs() {
                    for side in 0..2 {
                        let obs = env.observe(e, side)?;
                        let view = Observation { battle_id: obs.battle_id, request: obs.request.as_deref(), log: &obs.log };
                        encoder.encode_row(2 * e + side, &view, out)?;
                        if let Some(logs) = &mut logs { logs.push((obs.log, obs.prev_log)); }
                    }
                }
                encoder.match_partners(out);
                encoder.damage_features(out);
                Ok(logs)
            }).map_err(PyValueError::new_err)
        })
    }

    #[staticmethod]
    fn layout(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
        let d = PyDict::new(py);
        macro_rules! constants {
            ($($name:ident),* $(,)?) => { $(d.set_item(stringify!($name), $name)?;)* };
        }
        constants!(N_TOKENS, OWN, FOE_A, FOE_B, K_MOVES, K_ABILITIES, K_ITEMS, N_DYNAMIC, N_POKEMON_FLOATS,
            N_FIELD_FLOATS, N_IDS, N_ACTION_FEATURES, N_TOKEN_FEATURES, N_CODES, N_ACTIONS, PASS,
            I_SPECIES, I_ITEM, I_ABILITY, I_TERA, I_TYPE1, I_TYPE2, I_MOVE0, I_CAND_MOVES, I_CAND_ABILITIES,
            I_CAND_ITEMS, LEGACY_ACTION_FEATURES, LEGACY_TOKEN_FEATURES, REFERENCE_ACTION_FEATURES,
            REFERENCE_TOKEN_FEATURES, CACHE_LIMIT, STAT_SCALE);
        d.set_item("POKEMON_FLOAT_NAMES", POKEMON_FLOAT_NAMES.to_vec())?;
        d.set_item("FIELD_FLOAT_NAMES", FIELD_FLOAT_NAMES.to_vec())?;
        d.set_item("VOLATILES", VOLATILES.to_vec())?;
        d.set_item("TERRAINS", TERRAINS.to_vec())?;
        d.set_item("SIDE_CONDITIONS", SIDE_CONDITIONS.to_vec())?;
        d.set_item("TIMED_SIDE", TIMED_SIDE.to_vec())?;
        d.set_item("EXTREME_WEATHER", EXTREME_WEATHER.to_vec())?;
        d.set_item("STATUSES", STATUSES.to_vec())?;
        d.set_item("BOOSTS", BOOSTS.to_vec())?;
        for (key, names) in [("PF", POKEMON_FLOAT_NAMES.as_slice()), ("FF", FIELD_FLOAT_NAMES.as_slice())] {
            let columns = PyDict::new(py);
            for (i, name) in names.iter().enumerate() { columns.set_item(name, i)?; }
            d.set_item(key, columns)?;
        }
        Ok(d)
    }
}

struct SmartState {
    agent: encoder::smart::SmartAgent,
    last_rows: Option<usize>,
}

/// Stateful two-slot scripted player. `second` uses the rows and context of the preceding `first`.
#[pyclass(frozen, module = "numbrion")]
struct SmartAgent {
    state: Mutex<SmartState>,
}

#[pymethods]
impl SmartAgent {
    #[new]
    #[pyo3(signature = (dex_path, seed, temperature=0.0, noise=0.005, protect_skip=0.15, protect_share=(0.55, 0.85)))]
    fn new(py: Python<'_>, dex_path: PathBuf, seed: u64, temperature: f32, noise: f32,
        protect_skip: f32, protect_share: (f32, f32)) -> PyResult<Self>
    {
        if !temperature.is_finite() || temperature < 0.0 || !noise.is_finite() || noise < 0.0
            || !protect_skip.is_finite() || !(0.0..=1.0).contains(&protect_skip)
            || !protect_share.0.is_finite() || !protect_share.1.is_finite()
            || protect_share.0 < 0.0 || protect_share.1 > 1.0 || protect_share.0 > protect_share.1
        {
            return Err(PyValueError::new_err("temperature/noise must be finite and nonnegative; Protect probabilities must be ordered in [0, 1]"));
        }
        let config = SmartConfig { temperature, noise, protect_skip, protect_share };
        let agent = py.detach(|| {
            Ok::<_, String>(encoder::smart::SmartAgent::new(Dex::shared(&dex_path)?, seed, config))
        }).map_err(PyValueError::new_err)?;
        Ok(Self { state: Mutex::new(SmartState { agent, last_rows: None }) })
    }

    #[allow(clippy::too_many_arguments)]
    fn first<'py>(&self, py: Python<'py>, ids: &Bound<'py, PyAny>, floats: &Bound<'py, PyAny>,
        field: &Bound<'py, PyAny>, action_features: &Bound<'py, PyAny>, token_features: &Bound<'py, PyAny>,
        rows: &Bound<'py, PyAny>, mask0: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyArray1<i32>>>
    {
        let n = field.cast::<PyArrayDyn<f32>>().ok().filter(|a| a.ndim() == 2)
            .map(|a| a.shape()[0]).ok_or_else(|| PyValueError::new_err("field must be a float32 numpy array of shape [rows, N_FIELD_FLOATS]"))?;
        let ids = readable::<i64>(ids, "ids", "int64", &[n, N_TOKENS, N_IDS])?;
        let floats = readable::<f32>(floats, "floats", "float32", &[n, N_TOKENS, N_POKEMON_FLOATS])?;
        let field = readable::<f32>(field, "field", "float32", &[n, N_FIELD_FLOATS])?;
        let action_features = readable::<f32>(action_features, "action_features", "float32", &[n, 2, N_CODES, N_ACTION_FEATURES])?;
        let token_features = readable::<f32>(token_features, "token_features", "float32", &[n, N_TOKENS, N_TOKEN_FEATURES])?;
        let count = rows.cast::<PyArrayDyn<i64>>().ok().filter(|a| a.ndim() == 1).map(|a| a.shape()[0])
            .ok_or_else(|| PyValueError::new_err("rows must be a one-dimensional int64 numpy array"))?;
        let rows = readable::<i64>(rows, "rows", "int64", &[count])?;
        let rows: Vec<usize> = rows.as_slice()?.iter().map(|&r| {
            usize::try_from(r).ok().filter(|&r| r < n)
                .ok_or_else(|| PyValueError::new_err(format!("row index {r} outside 0..{n}")))
        }).collect::<PyResult<_>>()?;
        let mask = readable::<bool>(mask0, "mask0", "bool", &[count, N_ACTIONS])?;
        let obs = ObsRows { ids: ids.as_slice()?, floats: floats.as_slice()?, field: field.as_slice()?,
            action_features: action_features.as_slice()?, token_features: token_features.as_slice()? };
        let mask = mask.as_slice()?;
        let result = py.detach(|| {
            let mut state = lock(&self.state);
            let result = state.agent.first(&obs, &rows, mask);
            state.last_rows = Some(count);
            result
        });
        Ok(PyArray1::from_vec(py, result))
    }

    fn second<'py>(&self, py: Python<'py>, mask1: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyArray1<i32>>> {
        // Take the mutex after detaching: another thread holding it must be able to reacquire the GIL.
        let count = mask1.cast::<PyArrayDyn<bool>>().ok().filter(|a| a.ndim() == 2).map(|a| a.shape()[0])
            .ok_or_else(|| PyValueError::new_err("mask1 must be a bool numpy array of shape [n, 47]"))?;
        let mask = readable::<bool>(mask1, "mask1", "bool", &[count, N_ACTIONS])?;
        let mask = mask.as_slice()?;
        let result = py.detach(|| {
            let mut state = lock(&self.state);
            let expected = state.last_rows.ok_or_else(|| "second requires a preceding first call".to_string())?;
            if count != expected { return Err(format!("mask1 has {count} rows, expected {expected} from first")); }
            Ok(state.agent.second(mask))
        }).map_err(PyValueError::new_err)?;
        Ok(PyArray1::from_vec(py, result))
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ObsEncoder>()?;
    m.add_class::<SmartAgent>()?;
    m.add("ENCODER", true)?;
    Ok(())
}
