//! `GridExecutor`: batched one-decision search (pure Rust; `numbrion.GridExecutor` in `py.rs` wraps it).
//! See docs/design/TRAINING-API.md, "Search".
//!
//! A *cell* is `(world, p1 codes, p2 codes, seed)`. A persistent rayon pool runs the cells and every pool
//! thread owns one reusable worker `Game`. Per cell the worker restores the world's root snapshot
//! (`Game::restore_from`: no parsing, no allocation), optionally reseeds, submits both sides' typed
//! choices and runs to the cell's stop point. A result lands at its cell's index and depends only on the
//! root, the codes, the seed and the options: never on the thread count, the scheduling or on what a
//! worker ran before (restore resets every transient field, `Battle::restore_from`).
//!
//! Worlds: a call takes one root per world (`roots[k]`) and a world index per cell. Determinized worlds
//! (the opponent's hidden sets resampled) are ordinary roots built by the caller, so the executor does not
//! care how they were made; worlds may differ in teams, names and seeds, only the log mode must agree.

use crate::{
    action::N_ACTIONS,
    game::Game,
    mask::{Req, SideActions},
};
use engine::{ids::MonId, state::choices::RequestKind};
use rayon::prelude::*;
use std::sync::{Mutex, MutexGuard};

/// Where a cell stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// At the first decision boundary after the root choices: the next turn's move request, a mid-turn
    /// replacement request (U-turn, Eject Button, ...), an end-of-turn faint replacement, a Revival
    /// Blessing request, or the end of the battle. Exactly one `BatchEnv.step`.
    Boundary,
    /// Like `Boundary`, then answer every replacement (switch) request with Showdown's `default` choice
    /// (`Side.autoChoose`: the first eligible party members in order) until a move request or the end.
    Turn,
}

/// PRNG of the cells.
#[derive(Clone, Debug)]
pub enum Seeds {
    /// Keep the root's PRNG state: every cell sees the root's real future randomness (exact replay).
    Root,
    /// `reseed` every cell: one seed per cell, or a single seed for all cells (common random numbers).
    Fixed(Vec<[u16; 4]>),
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub stop: Stop,
    /// Each side's request JSON at the leaf.
    pub requests: bool,
    /// `mask0` / `mask1_any` at the leaf.
    pub masks: bool,
    /// Each side's player-view lines produced between the root and the leaf (`TextLog` roots only).
    pub logs: bool,
    /// Each leaf as an independent `Game` (one scratch allocation per cell).
    pub keep_leaves: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { stop: Stop::Boundary, requests: true, masks: false, logs: false, keep_leaves: false }
    }
}

/// Bounds the auto-answered replacement rounds of `Stop::Turn` (a turn has at most a few).
const MAX_STEPS: u8 = 32;

/// The leaf of one cell. Fields not asked for in `Options` stay empty.
#[derive(Default)]
pub struct Leaf {
    pub ended: bool,
    /// Winner (0/1) of a finished battle; -1 for a tie or a running battle.
    pub winner: i8,
    pub turn: i32,
    /// Choice rounds applied: 1, plus the replacement rounds auto-answered under `Stop::Turn`.
    pub steps: u8,
    /// `Req::code` per side: 0 none/wait, 1 move request, 2 replacement request.
    pub req: [u8; 2],
    pub needs: [bool; 2],
    pub mask0: [u64; 2],
    pub mask1_any: [u64; 2],
    pub requests: [Option<String>; 2],
    pub logs: [Vec<String>; 2],
    /// Boxed: a `Game` holds its ~38 KB `BattleState` inline, which would bloat every `Leaf`.
    pub game: Option<Box<Game>>,
}

/// Rejected input (`ValueError` in Python) or an engine failure on a mask-legal input (`RuntimeError`).
#[derive(Debug)]
pub enum GridError {
    Input(String),
    Engine(String),
}

impl std::fmt::Display for GridError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GridError::Input(e) | GridError::Engine(e) => f.write_str(e),
        }
    }
}

/// A root snapshot plus its decision information, computed once per call.
struct World {
    root: Game,
    acts: [SideActions; 2],
    party: [[MonId; 6]; 2],
}

pub struct GridExecutor {
    pool: rayon::ThreadPool,
    /// One reusable worker per pool thread, created on first use from a root.
    workers: Vec<Mutex<Option<Game>>>,
    /// Root snapshots of the current call (slots are reused across calls).
    worlds: Vec<World>,
}

/// A worker whose previous cell panicked may be mid-execution: drop it so it is rebuilt from a root.
fn lock_worker(m: &Mutex<Option<Game>>) -> MutexGuard<'_, Option<Game>> {
    m.lock().unwrap_or_else(|e| {
        m.clear_poison();
        let mut g = e.into_inner();
        *g = None;
        g
    })
}

impl GridExecutor {
    pub fn new(threads: usize) -> Result<GridExecutor, String> {
        let threads = threads.max(1);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .stack_size(16 << 20)
            .thread_name(|i| format!("numbrion-grid-{i}"))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(GridExecutor { pool, workers: (0..threads).map(|_| Mutex::new(None)).collect(), worlds: Vec::new() })
    }

    pub fn threads(&self) -> usize {
        self.workers.len()
    }

    /// Snapshot `root` as world `k` (restored into a reused slot, ~1 us). Worlds `0..k` must be staged.
    pub fn stage_root(&mut self, k: usize, root: &Game) -> Result<(), GridError> {
        if k > self.worlds.len() {
            return Err(GridError::Input(format!("world {k} staged before world {}", self.worlds.len())));
        }
        if !root.started() || root.ended() {
            let what = if root.ended() { "has ended" } else { "has not been started" };
            return Err(GridError::Input(format!("root of world {k} {what}")));
        }
        let acts = [root.side_actions(0), root.side_actions(1)];
        let party = [root.party(0), root.party(1)];
        // A side that already chose would let the other side's choice run the turn. Judge this from the
        // request itself: a choice that cannot be undone hides the side's actions from the mask.
        if let Some(s) = (0..2).find(|&s| has_chosen(root, s)) {
            return Err(GridError::Input(format!(
                "root of world {k}: side {s} has already chosen; roots must be taken before either side chooses"
            )));
        }
        if !acts.iter().any(SideActions::needs_action) {
            return Err(GridError::Input(format!("root of world {k} has no pending request")));
        }
        match self.worlds.get_mut(k) {
            Some(w) if w.root.is_text() == root.is_text() => {
                w.root.restore_from(root).map_err(GridError::Input)?;
                w.acts = acts;
                w.party = party;
            }
            Some(w) => *w = World { root: root.fork(false), acts, party },
            None => self.worlds.push(World { root: root.fork(false), acts, party }),
        }
        Ok(())
    }

    /// Stage `roots` and run the cells, see `run_staged`.
    pub fn run(
        &mut self,
        roots: &[&Game],
        world: &[u32],
        actions: &[i32],
        seeds: &Seeds,
        opts: &Options,
    ) -> Result<Vec<Leaf>, GridError> {
        for (k, r) in roots.iter().enumerate() {
            self.stage_root(k, r)?;
        }
        self.run_staged(roots.len(), world, actions, seeds, opts)
    }

    /// Run `actions.len() / 4` cells on the first `n_worlds` staged roots. `actions[(cell * 2 + side) * 2
    /// + slot]` are action codes; a side that needs no action at its root ignores them (use -1). `world`
    /// gives each cell's root (empty: world 0). Every code is validated against its root's masks before
    /// anything runs. Leaves come back in cell order.
    pub fn run_staged(
        &mut self,
        n_worlds: usize,
        world: &[u32],
        actions: &[i32],
        seeds: &Seeds,
        opts: &Options,
    ) -> Result<Vec<Leaf>, GridError> {
        let bad = |e: String| Err(GridError::Input(e));
        if n_worlds == 0 || n_worlds > self.worlds.len() {
            return bad(format!("{n_worlds} worlds requested, {} staged", self.worlds.len()));
        }
        if actions.len() % 4 != 0 {
            return bad(format!("actions must be [n, 2, 2], got {} entries", actions.len()));
        }
        let n = actions.len() / 4;
        if !world.is_empty() && world.len() != n {
            return bad(format!("world must have one entry per cell ({n}), got {}", world.len()));
        }
        if let Seeds::Fixed(s) = seeds
            && s.len() != 1
            && s.len() != n
        {
            return bad(format!("seeds must be one seed or one per cell ({n}), got {}", s.len()));
        }
        let worlds = &self.worlds[..n_worlds];
        let text = worlds[0].root.is_text();
        if worlds.iter().any(|w| w.root.is_text() != text) {
            return bad("all roots must use the same log mode".into());
        }
        if opts.logs && !text {
            return bad("logs need roots created with log=True".into());
        }
        for i in 0..n {
            let w = world.get(i).map_or(0, |&w| w as usize);
            let Some(wd) = worlds.get(w) else {
                return bad(format!("cell {i}: world {w} out of range 0..{n_worlds}"));
            };
            for s in 0..2 {
                let (a0, a1) = (actions[4 * i + 2 * s], actions[4 * i + 2 * s + 1]);
                let a = &wd.acts[s];
                if a.needs_action() && !(a0 >= 0 && a1 >= 0 && a.is_legal(a0 as usize, a1 as usize)) {
                    return bad(format!(
                        "cell {i} side {s}: action ({a0}, {a1}) is not legal in world {w} (request kind {}, mask0 {:#x})",
                        a.req.code(),
                        a.mask0()
                    ));
                }
            }
        }
        let workers = &self.workers;
        let cells: Vec<Result<Leaf, GridError>> = self.pool.install(|| {
            (0..n)
                .into_par_iter()
                .with_max_len(1)
                .map(|i| {
                    let w = &worlds[world.get(i).map_or(0, |&w| w as usize)];
                    let seed = match seeds {
                        Seeds::Root => None,
                        Seeds::Fixed(s) => Some(s[if s.len() == 1 { 0 } else { i }]),
                    };
                    // Pool threads are 0..threads; each locks only its own worker (never contended).
                    let t = rayon::current_thread_index().unwrap_or(0) % workers.len();
                    run_cell(&mut lock_worker(&workers[t]), w, i, &actions[4 * i..4 * i + 4], seed, opts)
                })
                .collect()
        });
        cells.into_iter().collect()
    }
}

fn run_cell(
    slot: &mut Option<Game>,
    w: &World,
    i: usize,
    codes: &[i32],
    seed: Option<[u16; 4]>,
    opts: &Options,
) -> Result<Leaf, GridError> {
    if slot.as_ref().is_some_and(|g| g.is_text() == w.root.is_text()) {
        slot.as_mut().unwrap().restore_from(&w.root).map_err(GridError::Engine)?;
    } else {
        *slot = Some(w.root.fork(false));
    }
    let g = slot.as_mut().unwrap();
    if let Some(seed) = seed {
        g.reseed(seed);
    }
    // Sides commit in a fixed order; the turn only runs once both have chosen, so the order is invisible.
    for s in 0..2 {
        let a = &w.acts[s];
        if !a.needs_action() {
            continue;
        }
        let (a0, a1) = (codes[2 * s] as usize, codes[2 * s + 1] as usize);
        let ch = a.choices(a0, a1, &w.party[s]).expect("validated against the root mask");
        g.choose_typed(s, &ch).map_err(|e| {
            GridError::Engine(format!("cell {i} side {s}: engine rejected a mask-legal action ({a0}, {a1}): {}", e.text))
        })?;
    }
    let mut steps = 1;
    let mut req = [g.request_kind(0), g.request_kind(1)];
    if opts.stop == Stop::Turn {
        while !g.ended() && steps < MAX_STEPS && req.contains(&Req::Switch) {
            for (s, r) in req.iter().enumerate() {
                if *r == Req::Switch {
                    g.choose(s, "default").map_err(|e| {
                        GridError::Engine(format!("cell {i} side {s}: default replacement rejected: {}", e.text))
                    })?;
                }
            }
            steps += 1;
            req = [g.request_kind(0), g.request_kind(1)];
        }
    }
    let mut leaf = Leaf { ended: g.ended(), winner: -1, turn: g.turn() as i32, steps, ..Leaf::default() };
    if let Some(w) = g.outcome().and_then(|o| o.winner) {
        leaf.winner = w as i8;
    }
    for (s, r) in req.iter().enumerate() {
        leaf.needs[s] = *r != Req::None;
        leaf.req[s] = r.code();
        if opts.masks && leaf.needs[s] {
            let a = g.side_actions(s);
            leaf.mask0[s] = a.mask0();
            leaf.mask1_any[s] = a.mask1_any(leaf.mask0[s]);
        }
        if opts.requests {
            leaf.requests[s] = g.request_json(s);
        }
        if opts.logs {
            leaf.logs[s] = g.drain_player(s);
        }
    }
    if opts.keep_leaves {
        leaf.game = Some(Box::new(g.fork(true)));
    }
    Ok(leaf)
}

/// Flat, C-ordered leaf arrays (`n` cells, `A` = `N_ACTIONS`), the layout `py.rs` hands to numpy.
#[derive(Default, Debug)]
pub struct GridArrays {
    pub n: usize,
    pub ended: Vec<u8>,
    pub winner: Vec<i8>,
    pub turn: Vec<i32>,
    pub steps: Vec<i32>,
    /// `[n, 2]`
    pub needs_action: Vec<u8>,
    /// `[n, 2]`
    pub request_kind: Vec<u8>,
    /// `[n, 2, A]`, only with `Options::masks`.
    pub mask0: Option<Vec<u8>>,
    /// `[n, 2, A]`, only with `Options::masks`.
    pub mask1_any: Option<Vec<u8>>,
}

impl GridArrays {
    pub fn from_leaves(leaves: &[Leaf], masks: bool) -> GridArrays {
        const A: usize = N_ACTIONS;
        let n = leaves.len();
        let mut o = GridArrays {
            n,
            ended: leaves.iter().map(|l| l.ended as u8).collect(),
            winner: leaves.iter().map(|l| l.winner).collect(),
            turn: leaves.iter().map(|l| l.turn).collect(),
            steps: leaves.iter().map(|l| l.steps as i32).collect(),
            needs_action: leaves.iter().flat_map(|l| l.needs.map(u8::from)).collect(),
            request_kind: leaves.iter().flat_map(|l| l.req).collect(),
            mask0: masks.then(|| vec![0; n * 2 * A]),
            mask1_any: masks.then(|| vec![0; n * 2 * A]),
        };
        if let (Some(m0), Some(m1)) = (&mut o.mask0, &mut o.mask1_any) {
            for (i, l) in leaves.iter().enumerate() {
                for s in 0..2 {
                    let k = (i * 2 + s) * A;
                    for a in crate::action::bits(l.mask0[s]) {
                        m0[k + a] = 1;
                    }
                    for a in crate::action::bits(l.mask1_any[s]) {
                        m1[k + a] = 1;
                    }
                }
            }
        }
        o
    }
}

/// The request asks `side` to act and it already holds a complete or irrevocable choice.
pub(crate) fn has_chosen(root: &Game, side: usize) -> bool {
    let state = root.state();
    let choice = &state.sides[side].choice;
    matches!(state.requests[side].kind, RequestKind::Move | RequestKind::Switch)
        && (choice.len >= 2 || choice.cant_undo)
}
