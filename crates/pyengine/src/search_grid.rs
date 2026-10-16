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
    action::{N_ACTIONS, PASS, SWITCH_BASE, FORCED_MOVE},
    game::Game,
    mask::{Req, SideActions},
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

/// Fused expansion, in input cell order. Policy rows are interleaved `[cell, side, ...]`.
pub struct ExpansionArrays {
    pub leaves: Vec<Leaf>,
    pub trackers: Vec<Option<[Tracker; 2]>>,
    pub policy: StateArrays,
    pub request_joint: Vec<u8>,
    pub engine_joint: Vec<u8>,
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

    /// Step to a boundary, retain live games and both observations, and encode policy inputs. Each side's
    /// tracker is cloned and fed exactly once, even when only its opponent needs to act at this boundary.
    /// Request masks use only the request; engine masks retain the world's hidden restrictions.
    #[allow(clippy::too_many_arguments)]
    pub fn expand_staged(
        &mut self,
        n_worlds: usize,
        world: &[u32],
        actions: &[i32],
        seeds: &Seeds,
        trackers: &[[&Tracker; 2]],
        stop: Stop,
    ) -> Result<ExpansionArrays, GridError> {
        if trackers.len() != 1 && trackers.len() != n_worlds {
            return Err(GridError::Input(format!("trackers must be one pair or one pair per root ({n_worlds}), got {}", trackers.len())));
        }
        for (k, pair) in trackers.iter().enumerate() {
            for (s, tracker) in pair.iter().enumerate() {
                if tracker.side != s {
                    return Err(GridError::Input(format!("trackers[{k}][{s}] tracks side {}, expected side {s}", tracker.side)));
                }
            }
        }
        let n = actions.len() / 4;
        let mut out = ExpansionArrays {
            leaves: Vec::new(),
            trackers: Vec::new(),
            policy: StateArrays {
                ids: vec![0; n * 2 * IDS_PER_ROW],
                floats: vec![0.0; n * 2 * FLOATS_PER_ROW],
                field: vec![0.0; n * 2 * FIELD_PER_ROW],
                token_features: vec![0.0; n * 2 * TOKEN_FEATURES_PER_ROW],
                action_features: Some(vec![0.0; n * 2 * ACTION_PER_ROW]),
                guard: Some(vec![false; n * 2 * GUARD_PER_ROW]),
            },
            request_joint: vec![0; n * 2 * N_ACTIONS * N_ACTIONS],
            engine_joint: vec![0; n * 2 * N_ACTIONS * N_ACTIONS],
        };
        let p = &mut out.policy;
        let mut rows = row_slices(&mut p.ids, &mut p.floats, &mut p.field, &mut p.token_features,
            p.action_features.as_mut(), p.guard.as_mut()).into_iter();
        let items: Vec<_> = out.request_joint.chunks_mut(2 * N_ACTIONS * N_ACTIONS)
            .zip(out.engine_joint.chunks_mut(2 * N_ACTIONS * N_ACTIONS))
            .map(|(request_joint, engine_joint)| ([rows.next().unwrap(), rows.next().unwrap()], request_joint, engine_joint))
            .collect();
        // The visitor owns the live leaf copy, so ended cells never allocate a retained game.
        let opts = Options { stop, requests: true, masks: false, logs: true, keep_leaves: false };
        let (dex, damage, scratch) = (&*self.dex, &*self.damage, &self.scratch);
        let cells = self.grid.run_staged_with(n_worlds, world, actions, seeds, &opts, items,
            |t, i, g, mut leaf, (rows, request_joint, engine_joint)| {
                if leaf.ended {
                    return Ok((leaf, None));
                }
                let pair = &trackers[if trackers.len() == 1 { 0 } else { world.get(i).copied().unwrap_or(0) as usize }];
                let sc = &mut *lock_scratch(&scratch[t % scratch.len()]);
                let mut states = [(*pair[0]).clone(), (*pair[1]).clone()];
                for (s, row) in rows.into_iter().enumerate() {
                    states[s].feed(&leaf.logs[s]);
                    if leaf.needs[s] {
                        // Parse the actual returned JSON, so masks and inputs use precisely the same request
                        // as the reference, including maybeTrapped/maybeDisabled omissions.
                        let request = parse_request(leaf.requests[s].as_deref())
                            .map_err(|e| GridError::Engine(format!("cell {i} side {s}: {e}")))?;
                        encode_full(dex, damage, &mut sc.cache, &states[s], &request, row)
                            .map_err(|e| GridError::Engine(format!("cell {i}: encoding side {s}'s leaf observation: {e}")))?;
                        let range = s * N_ACTIONS * N_ACTIONS..(s + 1) * N_ACTIONS * N_ACTIONS;
                        request_actions(&request).write_joint(&mut request_joint[range.clone()]);
                        g.side_actions(s).write_joint(&mut engine_joint[range]);
                    }
                }
                leaf.logs = Default::default();
                leaf.game = Some(Box::new(g.fork(true)));
                Ok((leaf, Some(states)))
            })?;
        (out.leaves, out.trackers) = cells.into_iter().unzip();
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

/// `training.protocol.Options`, using only fields visible in a request. Reuse the engine mask's joint
/// constraints, but reconstruct the per-slot options and replacement budgets from the request alone.
fn request_actions(request: &Json<'_>) -> SideActions {
    if !request.truthy() || request.at("wait").truthy() || request.at("teamPreview").truthy()
        || request.get("side").is_none()
        || (request.get("forceSwitch").is_none() && request.get("active").is_none()) {
        return SideActions::none();
    }
    let party = request.at("side").at("pokemon").items();
    let fainted = |mon: &Json<'_>| mon.at("condition").str_or_empty().ends_with("fnt");
    let bench = party.iter().enumerate().skip(2)
        .filter(|(_, mon)| !fainted(mon)).fold(0u64, |set, (i, _)| set | (1 << (SWITCH_BASE + i)));
    let mut out = SideActions::none();
    if let Some(flags) = request.get("forceSwitch") {
        out.req = Req::Switch;
        let flagged: [bool; 2] = std::array::from_fn(|k| flags.items().get(k).is_some_and(Json::truthy));
        let count = flagged.iter().filter(|&&f| f).count() as u8;
        let forced = count.min(bench.count_ones() as u8);
        out.legal.constraints.forced_switches = forced;
        out.legal.constraints.forced_passes = count - forced;
        let dead = party.iter().enumerate().filter(|(_, mon)| fainted(mon))
            .fold(0u64, |set, (i, _)| set | (1 << (SWITCH_BASE + i)));
        for k in 0..2 {
            if !flagged[k] {
                out.auto[k] = true;
                out.slot_set[k] = 1 << PASS;
            } else if party.get(k).is_some_and(|mon| mon.at("reviving").truthy()) {
                out.revival[k] = true;
                out.slot_set[k] = dead;
            } else {
                out.slot_set[k] = bench | if count > forced { 1 << PASS } else { 0 };
            }
        }
    } else {
        out.req = Req::Move;
        let active = request.at("active").items();
        for k in 0..2 {
            let mon = party.get(k);
            let entry = active.get(k);
            if mon.is_none_or(|mon| fainted(mon) || mon.at("commanding").truthy()) || entry.is_none_or(Json::is_null) {
                out.auto[k] = true;
                out.slot_set[k] = 1 << PASS;
                continue;
            }
            let entry = entry.unwrap();
            let moves = entry.at("moves").items();
            let moves = &moves[..moves.len().min(4)];
            let locked = moves.len() == 1 && moves[0].get("pp").is_none();
            let mut enabled = false;
            if !locked {
                for (j, m) in moves.iter().enumerate() {
                    if m.at("disabled").truthy() || m.get("pp").and_then(Json::as_f64).unwrap_or(1.0) <= 0.0 {
                        continue;
                    }
                    enabled = true;
                    for loc in -2..=2 {
                        if request_target(m.at("target").str_or_empty(), loc, k) {
                            let code = j * 10 + ((loc + 2) * 2) as usize;
                            out.slot_set[k] |= 1 << code;
                            if entry.at("canTerastallize").truthy() {
                                out.slot_set[k] |= 1 << (code + 1);
                            }
                        }
                    }
                }
            }
            if !enabled {
                out.slot_set[k] |= 1 << FORCED_MOVE;
            }
            if !entry.at("trapped").truthy() {
                out.slot_set[k] |= bench;
            }
        }
    }
    out
}

fn request_target(target: &str, loc: i32, position: usize) -> bool {
    let choosable = matches!(target, "normal" | "any" | "adjacentAlly" | "adjacentAllyOrSelf" | "adjacentFoe");
    if !choosable { return loc == 0; }
    if loc == 0 { return false; }
    let source = -(position as i32 + 1);
    let is_self = loc == source;
    let is_foe = loc > 0;
    let adjacent = if is_foe { (-(3 - loc) - source).abs() <= 1 } else { (loc - source).abs() == 1 };
    match target {
        "normal" => adjacent,
        "adjacentAlly" => adjacent && !is_foe,
        "adjacentAllyOrSelf" => (adjacent && !is_foe) || is_self,
        "adjacentFoe" => adjacent && is_foe,
        "any" => !is_self,
        _ => false,
    }
}

#[cfg(test)]
mod expansion_tests {
    use super::*;

    fn actions(text: &str) -> SideActions {
        request_actions(&Json::parse(text).unwrap())
    }

    #[test]
    fn request_none_and_wait() {
        for text in ["null", "{}", r#"{"wait":true,"side":{}}"#, r#"{"teamPreview":true,"side":{}}"#] {
            let a = actions(text);
            assert_eq!(a.req, Req::None);
            assert_eq!(a.mask0(), 0);
        }
    }

    #[test]
    fn request_hidden_trap_and_joint_constraints() {
        let a = actions(r#"{"side":{"pokemon":[{},{},{},{}]},"active":[
            {"moves":[{"pp":10,"target":"normal"}],"canTerastallize":"Normal","trapped":true},
            {"moves":[{"pp":10,"target":"adjacentAllyOrSelf"}],"canTerastallize":"Normal","maybeTrapped":true}] }"#);
        assert_eq!(a.slot_set[0], [0, 1, 6, 7, 8, 9].into_iter().fold(0, |s, c| s | 1 << c));
        assert_eq!(a.slot_set[1], [0, 1, 2, 3, 42, 43].into_iter().fold(0, |s, c| s | 1 << c));
        assert!(a.is_legal(6, 42)); // maybeTrapped never removes switches
        assert!(!a.is_legal(7, 1)); // two Tera choices
        assert!(a.is_legal(7, 0));
        let b = actions(r#"{"side":{"pokemon":[{},{},{}]},"active":[{"moves":[]},{"moves":[]}]}"#);
        assert!(b.is_legal(FORCED_MOVE, 42));
        assert!(!b.is_legal(42, 42));
    }

    #[test]
    fn request_forced_pass_and_revival_budgets() {
        let a = actions(r#"{"side":{"pokemon":[{"condition":"0 fnt"},{"condition":"0 fnt"},{}]},"forceSwitch":[true,true]}"#);
        assert!(a.is_legal(42, PASS));
        assert!(a.is_legal(PASS, 42));
        assert!(!a.is_legal(PASS, PASS));
        assert!(!a.is_legal(42, 42));
        let b = actions(r#"{"side":{"pokemon":[{"reviving":true},{"condition":"0 fnt"},{"condition":"0 fnt"}]},"forceSwitch":[true,false]}"#);
        assert!(b.is_legal(41, PASS));
        assert!(b.is_legal(42, PASS));
        assert!(!b.is_legal(PASS, PASS));
    }

    #[test]
    fn request_forced_moves_disabled_and_automatic_passes() {
        for moves in [r#"[{"target":"normal"}]"#, r#"[{"pp":0}]"#, r#"[{"pp":10,"disabled":true}]"#] {
            let text = format!(r#"{{"side":{{"pokemon":[{{}},{{"commanding":true}}]}},"active":[{{"moves":{moves},"canTerastallize":"Normal"}},{{}}]}}"#);
            let a = actions(&text);
            assert_eq!(a.slot_set, [1 << FORCED_MOVE, 1 << PASS]);
            assert!(a.is_legal(FORCED_MOVE, PASS));
        }
    }
}
