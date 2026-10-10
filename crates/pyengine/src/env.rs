//! `BatchEnv`: N independent battles stepped in parallel with auto-reset (pure Rust; the Python class in
//! `py.rs` is a thin wrapper). Battles run with `NoLog` unless `Config::log` asks for player log views.

use crate::{
    action::*,
    game::Game,
    mask::SideActions,
    pool::{SplitMix, TeamPool},
};
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnIllegal {
    /// `step` returns an error and applies nothing if any submitted action is not in the mask.
    Raise,
    /// An illegal side is given the engine's `default` choice and flagged in `illegal`.
    Default,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub n_envs: usize,
    pub seed: u64,
    pub threads: usize,
    /// Run `TextLog` battles so `observe` can return per-player log lines (slower than `NoLog`).
    pub log: bool,
    /// Also produce the full `[N_ACTIONS, N_ACTIONS]` joint mask per side and step.
    pub joint_mask: bool,
    pub on_illegal: OnIllegal,
}

/// Parameters of the built-in random sampler (modelled on the Showdown fixture generator's picker).
#[derive(Clone, Copy, Debug)]
pub struct SamplerParams {
    /// Probability of switching when a switch is legal in a move request.
    pub switch_prob: f64,
    /// Probability of terastallizing when the chosen move allows it.
    pub tera_prob: f64,
}

impl Default for SamplerParams {
    fn default() -> Self {
        SamplerParams { switch_prob: 0.1, tera_prob: 0.15 }
    }
}

/// Everything `step` reports about one environment.
#[derive(Clone, Copy, Debug, Default)]
struct EnvOut {
    reward: [f32; 2],
    done: bool,
    winner: i8,
    turn: i32,
    final_turns: i32,
    battle_id: i64,
    illegal: [bool; 2],
    req: [u8; 2],
    needs: [bool; 2],
    mask0: [u64; 2],
    mask1_any: [u64; 2],
}

impl EnvOut {
    fn fresh() -> Self {
        EnvOut { winner: -1, ..EnvOut::default() }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EnvStats {
    pub battles: u64,
    pub decisions: u64,
    pub boundaries: u64,
    pub turns: u64,
    pub illegal: u64,
}

struct Env {
    idx: usize,
    rng: SplitMix,
    pol: SplitMix,
    game: Game,
    battle_id: i64,
    acts: [SideActions; 2],
    prev_log: [Vec<String>; 2],
    out: EnvOut,
    joint: Vec<u8>,
    stats: EnvStats,
}

struct Shared<'a> {
    pool: &'a TeamPool,
    cfg: &'a Config,
}

pub struct BatchEnv {
    cfg: Config,
    pool: Arc<TeamPool>,
    envs: Vec<Env>,
    tp: rayon::ThreadPool,
}

/// Flat, C-ordered result arrays (`n` = number of environments, `A` = `N_ACTIONS`).
#[derive(Default, Debug)]
pub struct StepOut {
    pub n: usize,
    /// `[n, 2]` the side must submit a choice now.
    pub needs_action: Vec<u8>,
    /// `[n, 2]` 0 nothing/wait, 1 move request, 2 replacement (switch) request.
    pub request_kind: Vec<u8>,
    /// `[n, 2, A]` codes slot 0 may take (that extend to a legal joint choice).
    pub mask0: Vec<u8>,
    /// `[n, 2, A]` codes slot 1 may take after at least one legal slot-0 code (the union over slot 0).
    pub mask1_any: Vec<u8>,
    /// `[n, 2, A, A]` `joint[.., a0, a1]` legal pair, if requested.
    pub joint: Option<Vec<u8>>,
    /// `[n, 2]` +1 win, -1 loss, 0 otherwise (and for ties), only on the step the battle ended.
    pub reward: Vec<f32>,
    /// `[n]` the battle ended on this step (the arrays above describe the next, freshly started battle).
    pub done: Vec<u8>,
    /// `[n]` winner of the finished battle (0/1), -1 for a tie or no battle finished.
    pub winner: Vec<i8>,
    /// `[n]` turn counter of the battle now running.
    pub turn: Vec<i32>,
    /// `[n]` turns the finished battle lasted (0 where `done` is false).
    pub final_turns: Vec<i32>,
    /// `[n]` episode counter of the battle now running (increments at every auto-reset).
    pub battle_id: Vec<i64>,
    /// `[n, 2]` the submitted action was not legal and the engine default was used (`OnIllegal::Default`).
    pub illegal: Vec<u8>,
    /// Side choices applied by this step.
    pub decisions: u64,
}

#[derive(Debug)]
pub struct Observation {
    pub request: Option<String>,
    /// Lines of the current battle `side` has been shown since the last observation.
    pub log: Vec<String>,
    /// Unread lines of the battle that finished at the last auto-reset (win/tie lines etc.).
    pub prev_log: Vec<String>,
    pub turn: i32,
    pub battle_id: i64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RunStats {
    pub battles: u64,
    pub decisions: u64,
    pub boundaries: u64,
    pub turns: u64,
    pub illegal: u64,
}

fn bit_nth(mut m: u64, n: usize) -> usize {
    for _ in 0..n {
        m &= m - 1;
    }
    m.trailing_zeros() as usize
}

const SWITCH_BITS: u64 = ((1u64 << N_SWITCH) - 1) << SWITCH_BASE;
const MOVE_BITS: u64 = (1u64 << N_MOVE_ACTIONS) - 1;

/// Pick one code from a non-empty mask: switch with `switch_prob`, otherwise a uniformly random move slot,
/// then a uniform target, then Tera with `tera_prob` (when offered).
pub fn sample_from(mask: u64, rng: &mut SplitMix, p: &SamplerParams) -> usize {
    debug_assert!(mask != 0);
    let moves = mask & MOVE_BITS;
    let rest = mask & !MOVE_BITS;
    if moves == 0 {
        return bit_nth(rest, rng.below(rest.count_ones() as usize));
    }
    let switches = mask & SWITCH_BITS;
    if switches != 0 && rng.unit() < p.switch_prob {
        return bit_nth(switches, rng.below(switches.count_ones() as usize));
    }
    // A uniformly random move slot among those with a legal code.
    let mut slots = [0u8; N_MOVE_SLOTS];
    let mut n = 0;
    for s in 0..N_MOVE_SLOTS {
        if moves >> (s * N_TARGETS * 2) & 0x3ff != 0 {
            slots[n] = s as u8;
            n += 1;
        }
    }
    let s = slots[rng.below(n)] as usize;
    let sm = moves >> (s * N_TARGETS * 2) & 0x3ff;
    // Uniform target among those legal for this move (with or without Tera).
    let mut targets = [0u8; N_TARGETS];
    let mut nt = 0;
    for t in 0..N_TARGETS {
        if sm >> (t * 2) & 3 != 0 {
            targets[nt] = t as u8;
            nt += 1;
        }
    }
    let t = targets[rng.below(nt)] as usize;
    let tm = sm >> (t * 2) & 3;
    let tera = match tm {
        0b01 => false,
        0b10 => true,
        _ => rng.unit() < p.tera_prob,
    };
    s * N_TARGETS * 2 + t * 2 + tera as usize
}

impl Env {
    fn create(idx: usize, shared: &Shared) -> Result<Env, String> {
        let cfg = shared.cfg;
        let mut rng = SplitMix::stream(cfg.seed, idx as u64, 0);
        let game = Self::new_game(&mut rng, shared)?;
        let mut e = Env {
            idx,
            pol: SplitMix::stream(cfg.seed, idx as u64, 1),
            rng,
            game,
            battle_id: 0,
            acts: [SideActions::none(); 2],
            prev_log: [Vec::new(), Vec::new()],
            out: EnvOut::fresh(),
            joint: if cfg.joint_mask { vec![0; 2 * N_ACTIONS * N_ACTIONS] } else { Vec::new() },
            stats: EnvStats::default(),
        };
        e.refresh(shared);
        Ok(e)
    }

    /// Sample a team pair and a battle seed, build the battle and run it to its first decision.
    fn new_game(rng: &mut SplitMix, shared: &Shared) -> Result<Game, String> {
        let mut last = String::new();
        for _ in 0..100 {
            let i = rng.below(shared.pool.len());
            let j = rng.below(shared.pool.len());
            let seed = rng.battle_seed();
            let names = ["Player 1".to_string(), "Player 2".to_string()];
            let game = shared.pool.parsed(i).and_then(|p1| shared.pool.parsed(j).and_then(|p2|
                Game::from_team_defs(seed, [shared.pool.get(i).clone(), shared.pool.get(j).clone()],
                    [p1, p2], names, shared.cfg.log, false)));
            match game {
                Ok(mut g) => {
                    g.start().map_err(|e| format!("battle failed to start (teams {i} vs {j}, seed {seed:?}): {e}"))?;
                    return Ok(g);
                }
                Err(e) => last = format!("teams {i} vs {j}: {e}"),
            }
        }
        Err(format!("could not build a battle from the team pool: {last}"))
    }

    fn reset(&mut self, shared: &Shared) -> Result<(), String> {
        self.reset_game(shared)?;
        self.battle_id += 1;
        self.prev_log = [Vec::new(), Vec::new()];
        self.out = EnvOut::fresh();
        self.refresh(shared);
        Ok(())
    }

    fn reset_game(&mut self, shared: &Shared) -> Result<(), String> {
        let mut last = String::new();
        for _ in 0..100 {
            let i = self.rng.below(shared.pool.len());
            let j = self.rng.below(shared.pool.len());
            let seed = self.rng.battle_seed();
            let reset = shared.pool.parsed(i).and_then(|p1| shared.pool.parsed(j).and_then(|p2|
                self.game.reset_from_team_defs(seed, [shared.pool.get(i).clone(), shared.pool.get(j).clone()], [p1, p2])));
            match reset {
                Ok(()) => {
                    self.game.start().map_err(|e| format!("battle failed to start (teams {i} vs {j}, seed {seed:?}): {e}"))?;
                    return Ok(());
                }
                Err(e) => last = format!("teams {i} vs {j}: {e}"),
            }
        }
        Err(format!("could not build a battle from the team pool: {last}"))
    }

    /// Recompute the legal-action information for the current decision boundary.
    fn refresh(&mut self, shared: &Shared) {
        for s in 0..2 {
            let a = self.game.side_actions(s);
            self.acts[s] = a;
            self.out.needs[s] = a.needs_action();
            self.out.req[s] = a.req.code();
            if a.needs_action() {
                let m0 = a.mask0();
                self.out.mask0[s] = m0;
                let mut any = 0;
                for cls in CLASS_MASKS {
                    let members = m0 & cls;
                    if members != 0 {
                        any |= a.mask1(members.trailing_zeros() as usize);
                    }
                }
                self.out.mask1_any[s] = any;
                if shared.cfg.joint_mask {
                    a.write_joint(&mut self.joint[s * N_ACTIONS * N_ACTIONS..(s + 1) * N_ACTIONS * N_ACTIONS]);
                }
            } else {
                self.out.mask0[s] = 0;
                self.out.mask1_any[s] = 0;
                if shared.cfg.joint_mask {
                    self.joint[s * N_ACTIONS * N_ACTIONS..(s + 1) * N_ACTIONS * N_ACTIONS].fill(0);
                }
            }
        }
        self.out.turn = self.game.turn() as i32;
        self.out.battle_id = self.battle_id;
    }

    fn validate(&self, actions: &[i32]) -> Result<(), String> {
        for s in 0..2 {
            if !self.acts[s].needs_action() {
                continue;
            }
            let (a0, a1) = (actions[2 * s], actions[2 * s + 1]);
            let ok = a0 >= 0 && a1 >= 0 && self.acts[s].is_legal(a0 as usize, a1 as usize);
            if !ok {
                return Err(format!(
                    "env {} side {}: action ({}, {}) is not legal (request kind {}, mask0 {:#x})",
                    self.idx, s, a0, a1, self.out.req[s], self.out.mask0[s]
                ));
            }
        }
        Ok(())
    }

    /// Apply the submitted actions of every side that has to act, run the battle to its next decision, and
    /// auto-reset a finished battle.
    fn advance(&mut self, shared: &Shared, actions: &[i32]) -> Result<(), String> {
        self.out.reward = [0.0; 2];
        self.out.done = false;
        self.out.winner = -1;
        self.out.final_turns = 0;
        self.out.illegal = [false; 2];
        let party = [self.game.party(0), self.game.party(1)];
        let mut applied = 0;
        for s in 0..2 {
            if !self.acts[s].needs_action() {
                continue;
            }
            let (a0, a1) = (actions[2 * s], actions[2 * s + 1]);
            let choice = if a0 >= 0 && a1 >= 0 { self.acts[s].choices(a0 as usize, a1 as usize, &party[s]) } else { None };
            match choice {
                Some(ch) => {
                    if let Err(e) = self.game.choose_typed(s, &ch) {
                        return Err(format!(
                            "env {} side {}: engine rejected a mask-legal action ({a0}, {a1}): {}",
                            self.idx, s, e.text
                        ));
                    }
                }
                None => {
                    self.out.illegal[s] = true;
                    self.stats.illegal += 1;
                    if let Err(e) = self.game.choose(s, "default") {
                        return Err(format!("env {} side {}: default choice rejected: {}", self.idx, s, e.text));
                    }
                }
            }
            applied += 1;
        }
        if applied == 0 {
            return Err(format!("env {}: no side has a pending request", self.idx));
        }
        self.stats.decisions += applied;
        self.stats.boundaries += 1;
        if self.game.ended() {
            let o = self.game.outcome().expect("ended battle has an outcome");
            if let Some(w) = o.winner {
                self.out.reward[w] = 1.0;
                self.out.reward[1 - w] = -1.0;
                self.out.winner = w as i8;
            }
            self.out.final_turns = o.turns as i32;
            self.stats.battles += 1;
            self.stats.turns += o.turns as u64;
            if shared.cfg.log {
                for s in 0..2 {
                    let lines = self.game.drain_player(s);
                    self.prev_log[s].extend(lines);
                }
            }
            self.reset_game(shared)?;
            self.battle_id += 1;
            self.out.done = true;
        }
        self.refresh(shared);
        Ok(())
    }

    /// Random legal actions for the sides that have to act, `-1` elsewhere.
    fn sample(&mut self, p: &SamplerParams) -> [i32; 4] {
        let mut out = [-1; 4];
        for s in 0..2 {
            let a = &self.acts[s];
            if !a.needs_action() {
                continue;
            }
            let m0 = self.out.mask0[s];
            if m0 == 0 {
                continue;
            }
            let a0 = sample_from(m0, &mut self.pol, p);
            let m1 = a.mask1(a0);
            if m1 == 0 {
                continue;
            }
            out[2 * s] = a0 as i32;
            out[2 * s + 1] = sample_from(m1, &mut self.pol, p) as i32;
        }
        out
    }
}

impl BatchEnv {
    pub fn new(cfg: Config, pool: Arc<TeamPool>) -> Result<BatchEnv, String> {
        if cfg.n_envs == 0 {
            return Err("n_envs must be at least 1".into());
        }
        if pool.is_empty() {
            return Err("team pool is empty".into());
        }
        let tp = rayon::ThreadPoolBuilder::new()
            .num_threads(cfg.threads.max(1))
            .stack_size(16 << 20)
            .thread_name(|i| format!("numbrion-env-{i}"))
            .build()
            .map_err(|e| e.to_string())?;
        let shared = Shared { pool: &pool, cfg: &cfg };
        let envs: Result<Vec<Env>, String> = tp.install(|| (0..cfg.n_envs).into_par_iter().map(|i| Env::create(i, &shared)).collect());
        let envs = envs?;
        Ok(BatchEnv { cfg, pool, envs, tp })
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn n_envs(&self) -> usize {
        self.envs.len()
    }

    pub fn pool_len(&self) -> usize {
        self.pool.len()
    }

    /// Start a fresh battle in every environment.
    pub fn reset(&mut self) -> Result<StepOut, String> {
        let shared = Shared { pool: &self.pool, cfg: &self.cfg };
        let envs = &mut self.envs;
        let r: Result<Vec<()>, String> = self.tp.install(|| envs.par_iter_mut().map(|e| e.reset(&shared)).collect());
        r?;
        Ok(self.gather(0))
    }

    /// The current boundary's information without stepping.
    pub fn current(&self) -> StepOut {
        self.gather(0)
    }

    /// `actions[(env * 2 + side) * 2 + slot]`. Sides that need no action ignore their entries.
    pub fn step(&mut self, actions: &[i32]) -> Result<StepOut, String> {
        let n = self.envs.len();
        if actions.len() != n * 4 {
            return Err(format!("actions must have {} entries ([n_envs, 2, 2]), got {}", n * 4, actions.len()));
        }
        if self.cfg.on_illegal == OnIllegal::Raise {
            for e in &self.envs {
                e.validate(&actions[e.idx * 4..e.idx * 4 + 4])?;
            }
        }
        let before: u64 = self.envs.iter().map(|e| e.stats.decisions).sum();
        let shared = Shared { pool: &self.pool, cfg: &self.cfg };
        let envs = &mut self.envs;
        let r: Vec<Result<(), String>> = self
            .tp
            .install(|| envs.par_iter_mut().map(|e| e.advance(&shared, &actions[e.idx * 4..e.idx * 4 + 4])).collect());
        for x in r {
            x?;
        }
        let after: u64 = self.envs.iter().map(|e| e.stats.decisions).sum();
        Ok(self.gather(after - before))
    }

    fn gather(&self, decisions: u64) -> StepOut {
        const A: usize = N_ACTIONS;
        let n = self.envs.len();
        let mut o = StepOut {
            n,
            needs_action: vec![0; n * 2],
            request_kind: vec![0; n * 2],
            mask0: vec![0; n * 2 * A],
            mask1_any: vec![0; n * 2 * A],
            joint: self.cfg.joint_mask.then(|| vec![0; n * 2 * A * A]),
            reward: vec![0.0; n * 2],
            done: vec![0; n],
            winner: vec![-1; n],
            turn: vec![0; n],
            final_turns: vec![0; n],
            battle_id: vec![0; n],
            illegal: vec![0; n * 2],
            decisions,
        };
        for (i, e) in self.envs.iter().enumerate() {
            let eo = &e.out;
            o.done[i] = eo.done as u8;
            o.winner[i] = eo.winner;
            o.turn[i] = eo.turn;
            o.final_turns[i] = eo.final_turns;
            o.battle_id[i] = eo.battle_id;
            for s in 0..2 {
                let k = i * 2 + s;
                o.needs_action[k] = eo.needs[s] as u8;
                o.request_kind[k] = eo.req[s];
                o.reward[k] = eo.reward[s];
                o.illegal[k] = eo.illegal[s] as u8;
                for a in bits(eo.mask0[s]) {
                    o.mask0[k * A + a] = 1;
                }
                for a in bits(eo.mask1_any[s]) {
                    o.mask1_any[k * A + a] = 1;
                }
            }
            if let Some(j) = &mut o.joint {
                j[i * 2 * A * A..(i + 1) * 2 * A * A].copy_from_slice(&e.joint);
            }
        }
        o
    }

    /// `[n, 2, A]` codes slot 1 may take given slot 0 chose `a0[env * 2 + side]` (all false for sides that need
    /// no action or whose `a0` is not a legal slot-0 code).
    pub fn mask_slot1(&self, a0: &[i32]) -> Result<Vec<u8>, String> {
        let n = self.envs.len();
        if a0.len() != n * 2 {
            return Err(format!("a0 must have {} entries ([n_envs, 2]), got {}", n * 2, a0.len()));
        }
        let mut out = vec![0u8; n * 2 * N_ACTIONS];
        for (i, e) in self.envs.iter().enumerate() {
            for s in 0..2 {
                let a = a0[i * 2 + s];
                if a < 0 || !e.acts[s].needs_action() {
                    continue;
                }
                let k = i * 2 + s;
                for b in bits(e.acts[s].mask1(a as usize)) {
                    out[k * N_ACTIONS + b] = 1;
                }
            }
        }
        Ok(out)
    }

    /// Uniformly-structured random legal actions for every side that has to act (`-1` elsewhere),
    /// `[n, 2, 2]` flattened.
    pub fn random_actions(&mut self, p: &SamplerParams) -> Vec<i32> {
        let envs = &mut self.envs;
        let chunks: Vec<[i32; 4]> = self.tp.install(|| envs.par_iter_mut().map(|e| e.sample(p)).collect());
        chunks.into_iter().flatten().collect()
    }

    /// What `side` of environment `env` may observe: its request and the log lines it has been shown since the
    /// last call. Both are hidden-information safe by construction (request: the side's own; log: the player view).
    pub fn observe(&mut self, env: usize, side: usize) -> Result<Observation, String> {
        if env >= self.envs.len() || side >= 2 {
            return Err(format!("observe({env}, {side}) out of range"));
        }
        let e = &mut self.envs[env];
        Ok(Observation {
            request: e.game.request_json(side),
            log: e.game.drain_player(side),
            prev_log: std::mem::take(&mut e.prev_log[side]),
            turn: e.out.turn,
            battle_id: e.battle_id,
        })
    }

    /// A copy of the battle of environment `env` at its current decision boundary (for search).
    pub fn clone_game(&self, env: usize) -> Result<Game, String> {
        self.envs.get(env).map(|e| e.game.duplicate()).ok_or_else(|| format!("env {env} out of range"))
    }

    pub fn stats(&self) -> RunStats {
        let mut r = RunStats::default();
        for e in &self.envs {
            r.battles += e.stats.battles;
            r.decisions += e.stats.decisions;
            r.boundaries += e.stats.boundaries;
            r.turns += e.stats.turns;
            r.illegal += e.stats.illegal;
        }
        r
    }

    /// Benchmark/test driver that never leaves Rust: every environment plays random legal self-play until it
    /// has finished `battles_per_env` more battles. Uses the same code path as `step`.
    pub fn run_random(&mut self, battles_per_env: u64, p: &SamplerParams) -> Result<RunStats, String> {
        let before = self.stats();
        let shared = Shared { pool: &self.pool, cfg: &self.cfg };
        let envs = &mut self.envs;
        let r: Vec<Result<(), String>> = self.tp.install(|| {
            envs.par_iter_mut()
                .with_max_len(1)
                .map(|e| {
                    let target = e.stats.battles + battles_per_env;
                    while e.stats.battles < target {
                        let a = e.sample(p);
                        e.advance(&shared, &a)?;
                    }
                    Ok(())
                })
                .collect()
        });
        for x in r {
            x?;
        }
        let after = self.stats();
        Ok(RunStats {
            battles: after.battles - before.battles,
            decisions: after.decisions - before.decisions,
            boundaries: after.boundaries - before.boundaries,
            turns: after.turns - before.turns,
            illegal: after.illegal - before.illegal,
        })
    }
}
