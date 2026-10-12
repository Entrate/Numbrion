//! `SearchGrid`: `GridExecutor` cells whose leaves are encoded for the value network on the worker threads,
//! straight into flat arrays (pure Rust; `numbrion.SearchGrid` in `py_search_grid.rs` wraps it). See
//! docs/design/TRAINING-API.md, "Fused leaf encoding".
//!
//! A cell runs exactly like a `GridExecutor` cell (restore the world's root, reseed, both sides' codes, run to
//! the stop point). Then, on the same thread:
//!
//! * the **eval side** is the preferred side (the searching player) if it must act at the leaf, else the other
//!   side if that one must act, else none (-1: the battle ended);
//! * the eval side's observation is its root tracker, cloned and fed with its player-view lines from the root to
//!   the leaf, encoded with its leaf request: ids, floats, field and token features (action features and guard on
//!   request), bit for bit what `Encoder::encode_batch` gives a tracker with the same history;
//! * the **partner** (what the value head reads from the other side's row) is the other side's own tokens 0-5,
//!   from its tracker, its lines and its leaf request the same way (no damage features).
//!
//! Each pool thread owns a scratch tracker and a static-feature cache (`encoder::encode::Statics`), indexed like
//! the executor's worker battles, so results do not depend on the thread count or the scheduling.

use crate::{
    game::Game,
    search::{GridArrays, GridError, GridExecutor, Leaf, Options, Seeds, Stop},
};
use encoder::{
    Dex,
    damage::DamageFeatures,
    encode::{
        ACTION_PER_ROW, FIELD_PER_ROW, FLOATS_PER_ROW, GUARD_PER_ROW, IDS_PER_ROW, RowOutputs, Statics,
        TOKEN_FEATURES_PER_ROW, encode_own_tokens, encode_tracker, parse_request,
    },
    json::Json,
    layout::{N_IDS, N_POKEMON_FLOATS},
    tracker::Tracker,
};
use rayon::prelude::*;
use std::sync::{Arc, Mutex, MutexGuard};

/// Own tokens of the partner row: `[6 * N_IDS]` ids and `[6 * N_POKEMON_FLOATS]` floats per cell.
pub const PARTNER_IDS_PER_ROW: usize = 6 * N_IDS;
pub const PARTNER_FLOATS_PER_ROW: usize = 6 * N_POKEMON_FLOATS;

/// What a `run` computes besides the value inputs.
#[derive(Clone, Copy, Debug)]
pub struct LeafOptions {
    pub stop: Stop,
    /// `mask0` / `mask1_any` at the leaf.
    pub masks: bool,
    /// The eval side's `action_features` (what the policy reads; the value head does not).
    pub action_features: bool,
    /// The eval side's `guard`.
    pub guard: bool,
}

impl Default for LeafOptions {
    fn default() -> Self {
        LeafOptions { stop: Stop::Boundary, masks: false, action_features: false, guard: false }
    }
}

/// Flat, C-ordered leaf arrays (`n` cells), zero where `eval_side` is -1.
#[derive(Default, Debug)]
pub struct LeafArrays {
    /// `ended`, `winner`, `turn`, `steps`, `needs_action`, `request_kind` (and masks), as `GridExecutor` returns.
    pub grid: GridArrays,
    /// `[n]`: the side whose observation was encoded, -1 when the battle ended.
    pub eval_side: Vec<i8>,
    /// `[n, N_TOKENS, N_IDS]`
    pub ids: Vec<i64>,
    /// `[n, N_TOKENS, N_POKEMON_FLOATS]`
    pub floats: Vec<f32>,
    /// `[n, N_FIELD_FLOATS]`
    pub field: Vec<f32>,
    /// `[n, N_TOKENS, N_TOKEN_FEATURES]`
    pub token_features: Vec<f32>,
    /// `[n, 6, N_IDS]`: the other side's own tokens.
    pub partner_ids: Vec<i64>,
    /// `[n, 6, N_POKEMON_FLOATS]`
    pub partner_floats: Vec<f32>,
    /// `[n, 2, N_CODES, N_ACTION_FEATURES]` with `LeafOptions::action_features`.
    pub action_features: Option<Vec<f32>>,
    /// `[n, 2, 4, 2]` with `LeafOptions::guard`.
    pub guard: Option<Vec<bool>>,
}

/// One hypothetical observation for `SearchGrid::encode_states`: a tracker state, the request it is encoded with
/// (None or "" for none) and lines fed to a copy of the tracker first.
pub struct StateInput<'a, S: AsRef<str>> {
    pub tracker: &'a Tracker,
    pub request: Option<&'a str>,
    pub lines: &'a [S],
}

/// Flat arrays of `encode_states` (`n` rows), the `Outputs` layout of the encoder.
#[derive(Default, Debug)]
pub struct StateArrays {
    pub ids: Vec<i64>,
    pub floats: Vec<f32>,
    pub field: Vec<f32>,
    pub token_features: Vec<f32>,
    pub action_features: Option<Vec<f32>>,
    pub guard: Option<Vec<bool>>,
}

/// Per pool thread: the scratch tracker a cell's root state is copied into and the static-feature cache.
struct Scratch {
    cache: Statics,
    tracker: Tracker,
}

/// One cell's (or one state's) slices of the output arrays.
struct RowSlices<'a> {
    ids: &'a mut [i64],
    floats: &'a mut [f32],
    field: &'a mut [f32],
    token_features: &'a mut [f32],
    action_features: Option<&'a mut [f32]>,
    guard: Option<&'a mut [bool]>,
}

struct CellSlices<'a> {
    eval_side: &'a mut i8,
    row: RowSlices<'a>,
    partner_ids: &'a mut [i64],
    partner_floats: &'a mut [f32],
}

fn lock_scratch(m: &Mutex<Scratch>) -> MutexGuard<'_, Scratch> {
    // Everything in the scratch is overwritten before use; a poisoned one is still consistent.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `n` row slices of the outputs (action features and guard only when allocated).
fn row_slices<'a>(
    ids: &'a mut [i64],
    floats: &'a mut [f32],
    field: &'a mut [f32],
    token_features: &'a mut [f32],
    action_features: Option<&'a mut Vec<f32>>,
    guard: Option<&'a mut Vec<bool>>,
) -> Vec<RowSlices<'a>> {
    let mut action = action_features.map(|v| v.chunks_mut(ACTION_PER_ROW));
    let mut guard = guard.map(|v| v.chunks_mut(GUARD_PER_ROW));
    ids.chunks_mut(IDS_PER_ROW)
        .zip(floats.chunks_mut(FLOATS_PER_ROW))
        .zip(field.chunks_mut(FIELD_PER_ROW))
        .zip(token_features.chunks_mut(TOKEN_FEATURES_PER_ROW))
        .map(|(((ids, floats), field), token_features)| RowSlices {
            ids,
            floats,
            field,
            token_features,
            action_features: action.as_mut().and_then(Iterator::next),
            guard: guard.as_mut().and_then(Iterator::next),
        })
        .collect()
}

pub struct SearchGrid {
    grid: GridExecutor,
    dex: Arc<Dex>,
    damage: Arc<DamageFeatures>,
    scratch: Vec<Mutex<Scratch>>,
}

impl SearchGrid {
    pub fn new(dex: Arc<Dex>, threads: usize) -> Result<SearchGrid, String> {
        let grid = GridExecutor::new(threads)?;
        let damage = Arc::new(DamageFeatures::new(&dex));
        let scratch = (0..grid.threads())
            .map(|_| Mutex::new(Scratch { cache: Statics::default(), tracker: Tracker::new(0) }))
            .collect();
        Ok(SearchGrid { grid, dex, damage, scratch })
    }

    pub fn threads(&self) -> usize {
        self.grid.threads()
    }

    pub fn dex(&self) -> &Arc<Dex> {
        &self.dex
    }

    /// Snapshot `root` as world `k`, see `GridExecutor::stage_root`.
    pub fn stage_root(&mut self, k: usize, root: &Game) -> Result<(), GridError> {
        self.grid.stage_root(k, root)
    }

    /// Stage `roots` and run the cells, see `run_staged`.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        roots: &[&Game],
        world: &[u32],
        actions: &[i32],
        seeds: &Seeds,
        trackers: &[[&Tracker; 2]],
        prefer: usize,
        opts: &LeafOptions,
    ) -> Result<LeafArrays, GridError> {
        for (k, r) in roots.iter().enumerate() {
            self.stage_root(k, r)?;
        }
        self.run_staged(roots.len(), world, actions, seeds, trackers, prefer, opts)
    }

    /// `GridExecutor::run_staged` (same inputs and validation; roots need `log=True`), then each leaf encoded for
    /// the value network. `trackers` are the observation trackers of sides 0 and 1 at the root (`[k][side]`): one
    /// pair for every world, or one pair per world. `prefer` is the searching player's side: the eval side when it
    /// must act at the leaf.
    #[allow(clippy::too_many_arguments)]
    pub fn run_staged(
        &mut self,
        n_worlds: usize,
        world: &[u32],
        actions: &[i32],
        seeds: &Seeds,
        trackers: &[[&Tracker; 2]],
        prefer: usize,
        opts: &LeafOptions,
    ) -> Result<LeafArrays, GridError> {
        let bad = |e: String| Err(GridError::Input(e));
        if prefer > 1 {
            return bad(format!("eval_side must be 0 or 1, got {prefer}"));
        }
        if trackers.len() != 1 && trackers.len() != n_worlds {
            return bad(format!("trackers must be one pair or one pair per root ({n_worlds}), got {}", trackers.len()));
        }
        for (k, pair) in trackers.iter().enumerate() {
            for (s, t) in pair.iter().enumerate() {
                if t.side != s {
                    return bad(format!("trackers[{k}][{s}] tracks side {}, expected side {s}", t.side));
                }
            }
        }
        let n = actions.len() / 4;
        let mut out = LeafArrays {
            eval_side: vec![-1; n],
            ids: vec![0; n * IDS_PER_ROW],
            floats: vec![0.0; n * FLOATS_PER_ROW],
            field: vec![0.0; n * FIELD_PER_ROW],
            token_features: vec![0.0; n * TOKEN_FEATURES_PER_ROW],
            partner_ids: vec![0; n * PARTNER_IDS_PER_ROW],
            partner_floats: vec![0.0; n * PARTNER_FLOATS_PER_ROW],
            action_features: opts.action_features.then(|| vec![0.0; n * ACTION_PER_ROW]),
            guard: opts.guard.then(|| vec![false; n * GUARD_PER_ROW]),
            ..LeafArrays::default()
        };
        let rows = row_slices(&mut out.ids, &mut out.floats, &mut out.field, &mut out.token_features,
            out.action_features.as_mut(), out.guard.as_mut());
        let items: Vec<CellSlices> = rows
            .into_iter()
            .zip(out.eval_side.iter_mut())
            .zip(out.partner_ids.chunks_mut(PARTNER_IDS_PER_ROW))
            .zip(out.partner_floats.chunks_mut(PARTNER_FLOATS_PER_ROW))
            .map(|(((row, eval_side), partner_ids), partner_floats)| CellSlices { eval_side, row, partner_ids, partner_floats })
            .collect();
        // The leaf's player views come back in `leaf.logs` (moved, not copied); requests are written by `visit`.
        let grid_opts = Options { stop: opts.stop, requests: false, masks: opts.masks, logs: true, keep_leaves: false };
        let (dex, damage, scratch) = (&*self.dex, &*self.damage, &self.scratch);
        let leaves = self.grid.run_staged_with(n_worlds, world, actions, seeds, &grid_opts, items,
            |t, i, g: &mut Game, mut leaf: Leaf, cell: CellSlices| {
                let pair = &trackers[if trackers.len() == 1 { 0 } else { world.get(i).map_or(0, |&w| w as usize) }];
                let eval = [prefer, 1 - prefer].into_iter().find(|&s| leaf.needs[s]);
                if let Some(e) = eval {
                    *cell.eval_side = e as i8;
                    let sc = &mut *lock_scratch(&scratch[t % scratch.len()]);
                    let logs = std::mem::take(&mut leaf.logs);
                    let fail = |side: usize, e: String| GridError::Engine(format!("cell {i}: encoding side {side}'s leaf observation: {e}"));
                    encode_row(dex, damage, sc, pair[e], &logs[e], g, e, cell.row).map_err(|err| fail(e, err))?;
                    let p = 1 - e;
                    let own = RowOutputs { ids: cell.partner_ids, floats: cell.partner_floats, field: None, fainted: None, guard: None };
                    encode_own(dex, sc, pair[p], &logs[p], g, p, own).map_err(|err| fail(p, err))?;
                }
                leaf.logs = Default::default();
                Ok(leaf)
            })?;
        out.grid = GridArrays::from_leaves(&leaves, opts.masks);
        Ok(out)
    }

    /// Encode hypothetical observations (e.g. the opponent's at each world's root): per state, a copy of its tracker
    /// fed with its lines, encoded with its request, plus damage features, exactly as `Encoder::encode_batch` would.
    /// Runs on the pool's threads.
    pub fn encode_states<S: AsRef<str> + Sync>(&self, states: &[StateInput<'_, S>], action_features: bool, guard: bool)
        -> Result<StateArrays, String>
    {
        let n = states.len();
        let mut out = StateArrays {
            ids: vec![0; n * IDS_PER_ROW],
            floats: vec![0.0; n * FLOATS_PER_ROW],
            field: vec![0.0; n * FIELD_PER_ROW],
            token_features: vec![0.0; n * TOKEN_FEATURES_PER_ROW],
            action_features: action_features.then(|| vec![0.0; n * ACTION_PER_ROW]),
            guard: guard.then(|| vec![false; n * GUARD_PER_ROW]),
        };
        let rows = row_slices(&mut out.ids, &mut out.floats, &mut out.field, &mut out.token_features,
            out.action_features.as_mut(), out.guard.as_mut());
        let (dex, damage, scratch) = (&*self.dex, &*self.damage, &self.scratch);
        let results: Vec<Result<(), String>> = self.grid.pool().install(|| {
            rows.into_par_iter()
                .zip(states.par_iter())
                .enumerate()
                .with_max_len(1)
                .map(|(i, (row, state))| {
                    let t = rayon::current_thread_index().unwrap_or(0) % scratch.len();
                    let sc = &mut *lock_scratch(&scratch[t]);
                    sc.tracker.clone_from(state.tracker);
                    sc.tracker.feed(state.lines);
                    let request = parse_request(state.request).map_err(|e| format!("state {i}: {e}"))?;
                    encode_full(dex, damage, &mut sc.cache, &sc.tracker, &request, row).map_err(|e| format!("state {i}: {e}"))
                })
                .collect()
        });
        results.into_iter().collect::<Result<(), String>>()?;
        Ok(out)
    }
}

/// The full row of `tracker` + `request` with its damage features.
fn encode_full(dex: &Dex, damage: &DamageFeatures, cache: &mut Statics, tracker: &Tracker, request: &Json,
    row: RowSlices<'_>) -> Result<(), String>
{
    let RowSlices { ids, floats, field, token_features, action_features, guard } = row;
    let out = RowOutputs { ids: &mut *ids, floats: &mut *floats, field: Some(&mut *field), fainted: None, guard };
    encode_tracker(dex, cache, tracker, request, out, None)?;
    match action_features {
        Some(action) => damage.row(ids, floats, field, action, token_features),
        None => damage.token_row(ids, floats, field, token_features),
    }
    Ok(())
}

/// `side`'s leaf request as `parse_request(request_json)` gives it (an empty object after the end), built from the
/// engine's request data without text (`request_tree.rs`).
fn leaf_request(g: &Game, side: usize) -> Json<'_> {
    g.request_tree(side).unwrap_or(Json::Obj(Vec::new()))
}

/// The leaf observation of `side`: its root tracker fed with `lines`, its leaf request, the full row.
#[allow(clippy::too_many_arguments)]
fn encode_row(dex: &Dex, damage: &DamageFeatures, sc: &mut Scratch, root: &Tracker, lines: &[String], g: &Game,
    side: usize, row: RowSlices<'_>) -> Result<(), String>
{
    sc.tracker.clone_from(root);
    sc.tracker.feed(lines);
    encode_full(dex, damage, &mut sc.cache, &sc.tracker, &leaf_request(g, side), row)
}

/// The own tokens 0-5 of `side`'s leaf observation.
fn encode_own(dex: &Dex, sc: &mut Scratch, root: &Tracker, lines: &[String], g: &Game, side: usize,
    out: RowOutputs<'_>) -> Result<(), String>
{
    sc.tracker.clone_from(root);
    sc.tracker.feed(lines);
    encode_own_tokens(dex, &mut sc.cache, &sc.tracker, &leaf_request(g, side), out)
}
