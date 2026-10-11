//! Profile-collection workload for PGO builds (`tools/pgo.sh`, `tools/pgo.ps1`, docs/design/PGO.md).
//!
//! It lives in the library, not in a separate binary, because a profile only applies to code compiled
//! exactly like the profiled build: the wheel's `_numbrion` crate is maturin's `cdylib` LTO root, and most
//! engine code (`Battle<L>` is generic over the log sink) is instantiated inside it, so its symbol names
//! carry `_numbrion`'s crate hash. An instrumented `cargo build` of an example never produces those names.
//! The scripts therefore run this workload from an instrumented wheel (`_numbrion._pgo_workload`); the
//! `pgo_workload` example runs the same code from cargo (smoke test, timing, difftest-style profiles).
//!
//! Phases, each a share of `seconds`:
//! - `selfplay-log` (25%): a TextLog `BatchEnv` driven the way the trainers drive it: `random_actions`,
//!   `mask_slot1`, `step`, then `observe` (request JSON + player-view lines) for every env and side.
//! - `selfplay-nolog` (15%): a NoLog `BatchEnv`: `run_random`, a `step` loop and `clone_game`.
//! - `selfplay-smart` (15%): encode every row at every boundary, including partner matching and damage
//!   features, then play both slots with `SmartAgent` and the engine's conditional legal masks.
//! - `grid` (30%): `GridExecutor` over mid-battle roots (NoLog and TextLog): both stop modes, every cell
//!   reseeded, requests on (masks and logs on alternate calls), plus multi-world calls on determinized worlds.
//! - `primitives` (15%): per cell `Game::fork` (clone), `restore_from`, or `restore_from` +
//!   `replace_hidden_set` of every hidden opponent set, each followed by the cell's turn and both requests.

use crate::{
    env::{BatchEnv, Config, OnIllegal, SamplerParams, sample_from},
    game::Game,
    mask::Req,
    pool::{SplitMix, TeamPool},
    search::{GridExecutor, Options, Seeds, Stop},
};
use encoder::{
    Dex,
    encode::{Encoder, Observation, Outputs, IDS_PER_ROW, FLOATS_PER_ROW, FIELD_PER_ROW, FOE_MATCH_PER_ROW,
        FAINTED_PER_ROW, GUARD_PER_ROW, ACTION_PER_ROW, TOKEN_FEATURES_PER_ROW},
    smart::{ObsRows, SmartAgent, SmartConfig},
};
use std::{
    fmt::Write,
    hint::black_box,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

struct Root {
    game: Game,
    /// `[cells, 2, 2]` codes: 10 x 10 joint actions per side.
    actions: Vec<i32>,
    seeds: Vec<[u16; 4]>,
    /// Packed-team index and donor set of every hidden p2 Pokemon (viewer p1) that `replace_hidden_set` accepts.
    swaps: Vec<(usize, String)>,
    /// The root plus three determinized worlds (hidden p2 sets resampled).
    worlds: Vec<Game>,
}

/// Up to `k` distinct legal pairs of one side (repeats if it has fewer).
fn pairs(g: &Game, side: usize, rng: &mut SplitMix, k: usize) -> Vec<[i32; 2]> {
    let a = g.side_actions(side);
    let p = SamplerParams { switch_prob: 0.2, tera_prob: 0.2 };
    let mut out: Vec<[i32; 2]> = Vec::new();
    for _ in 0..200 {
        let a0 = sample_from(a.mask0(), rng, &p);
        let pair = [a0 as i32, sample_from(a.mask1(a0), rng, &p) as i32];
        if !out.contains(&pair) {
            out.push(pair);
        }
        if out.len() == k {
            break;
        }
    }
    let distinct = out.len();
    for i in distinct..k {
        out.push(out[i % distinct]);
    }
    out
}

/// A donor set accepted by `replace_hidden_set` for every hidden p2 index of `root` (viewer p1).
fn donor_swaps(root: &Game, pool: &TeamPool, rng: &mut SplitMix) -> Vec<(usize, String)> {
    let mut trial = root.fork(false);
    let mut out = Vec::new();
    for index in trial.hidden_team_indices(0) {
        for _ in 0..4 {
            let donor = pool.get(rng.below(pool.len()));
            let set = donor.split(']').nth(rng.below(6)).unwrap_or_default().to_owned();
            // Refusals (BattleStart species, gender draws) keep the real set; try another donor.
            if trial.replace_hidden_set(0, index, &set).is_ok() {
                out.push((index, set));
                break;
            }
        }
    }
    out
}

/// Mid-battle turn boundaries (turn 3-10 of random self-play) where both sides choose.
fn roots(pool: &TeamPool, count: usize, log: bool, seed: u64) -> Result<Vec<Root>, String> {
    let mut rng = SplitMix::new(seed);
    let p = SamplerParams::default();
    let mut out = Vec::new();
    for _ in 0..100 * count + 100 {
        if out.len() == count {
            return Ok(out);
        }
        let (i, j) = (rng.below(pool.len()), rng.below(pool.len()));
        let Ok(mut g) = Game::new(rng.battle_seed(), pool.get(i).clone(), pool.get(j).clone(),
            ["Alice".into(), "Bob".into()], log, false) else { continue };
        if g.start().is_err() {
            continue;
        }
        let target = 3 + rng.below(8) as u32;
        while !g.ended() && g.turn() < target {
            // Both masks before either side chooses: the second choice may run the turn.
            let sides = [g.side_actions(0), g.side_actions(1)];
            let party = [g.party(0), g.party(1)];
            for (s, a) in sides.iter().enumerate() {
                if a.needs_action() {
                    let a0 = sample_from(a.mask0(), &mut rng, &p);
                    let a1 = sample_from(a.mask1(a0), &mut rng, &p);
                    let ch = a.choices(a0, a1, &party[s]).ok_or("sampled codes have no engine choice")?;
                    g.choose_typed(s, &ch).map_err(|e| e.text)?;
                }
            }
        }
        if g.ended() || !(0..2).all(|s| g.side_actions(s).req == Req::Move) {
            continue;
        }
        let (p1, p2) = (pairs(&g, 0, &mut rng, 10), pairs(&g, 1, &mut rng, 10));
        let actions: Vec<i32> = p1.iter().flat_map(|a| p2.iter().flat_map(move |b| [a[0], a[1], b[0], b[1]])).collect();
        let seeds = (0..actions.len() / 4).map(|_| rng.battle_seed()).collect();
        g.drain_player(0);
        g.drain_player(1);
        let swaps = donor_swaps(&g, pool, &mut rng);
        let mut worlds = vec![g.fork(false)];
        for _ in 0..3 {
            let mut w = g.fork(false);
            for (index, set) in donor_swaps(&g, pool, &mut rng) {
                w.replace_hidden_set(0, index, &set)?;
            }
            worlds.push(w);
        }
        out.push(Root { game: g, actions, seeds, swaps, worlds });
    }
    if out.len() == count { Ok(out) } else { Err(format!("could not build {count} mid-battle roots from the team pool")) }
}

/// One cell the sequential way on `g` (already a copy of the root): reseed, both choices, both requests.
fn cell(g: &mut Game, codes: &[i32], seed: [u16; 4]) -> Result<usize, String> {
    g.reseed(seed);
    let sides = [g.side_actions(0), g.side_actions(1)];
    let party = [g.party(0), g.party(1)];
    for s in 0..2 {
        let ch = sides[s].choices(codes[2 * s] as usize, codes[2 * s + 1] as usize, &party[s])
            .ok_or("root-legal codes have no engine choice")?;
        g.choose_typed(s, &ch).map_err(|e| e.text)?;
    }
    let mut n = g.request_json(0).map_or(0, |r| r.len()) + g.request_json(1).map_or(0, |r| r.len());
    if g.is_text() {
        n += g.drain_player(0).len() + g.drain_player(1).len();
    }
    Ok(n)
}

#[derive(Default)]
struct Tally {
    calls: u64,
    items: u64,
    sink: usize,
}

fn selfplay(pool: &Arc<TeamPool>, threads: usize, log: bool, budget: Duration) -> Result<Tally, String> {
    let cfg = Config { n_envs: 32, seed: if log { 11 } else { 12 }, threads, log, joint_mask: false, on_illegal: OnIllegal::Raise };
    let n = cfg.n_envs;
    let mut env = BatchEnv::new(cfg, Arc::clone(pool))?;
    let p = SamplerParams::default();
    let mut t = Tally::default();
    let start = Instant::now();
    env.reset()?;
    while start.elapsed() < budget {
        if !log && t.calls % 4 == 3 {
            // The all-Rust driver: one more finished battle per env.
            t.items += env.run_random(1, &p)?.battles;
        } else {
            for _ in 0..16 {
                let actions = env.random_actions(&p);
                let a0: Vec<i32> = actions.chunks(2).map(|c| c[0]).collect();
                t.sink += env.mask_slot1(&a0)?.iter().map(|&x| x as usize).sum::<usize>();
                let out = env.step(&actions)?;
                t.items += out.done.iter().map(|&d| d as u64).sum::<u64>();
                if log {
                    for e in 0..n {
                        for s in 0..2 {
                            let o = env.observe(e, s)?;
                            t.sink += o.request.map_or(0, |r| r.len()) + o.log.len() + o.prev_log.len();
                        }
                    }
                }
            }
            t.sink += env.clone_game(t.calls as usize % n)?.turn() as usize;
        }
        t.calls += 1;
    }
    Ok(t)
}

fn selfplay_smart(pool: &Arc<TeamPool>, threads: usize, budget: Duration) -> Result<Tally, String> {
    let cfg = Config { n_envs: 16, seed: 13, threads, log: true, joint_mask: false, on_illegal: OnIllegal::Raise };
    let rows: Vec<usize> = (0..2 * cfg.n_envs).collect();
    let n = rows.len();
    let mut env = BatchEnv::new(cfg, Arc::clone(pool))?;
    // Both the cargo example and wheel workload run from a source checkout (tools/pgo.*).
    let dex = Dex::shared(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/training/dex.json")))?;
    let mut encoder = Encoder::new(dex.clone(), n);
    let mut smart = SmartAgent::new(dex, 14, SmartConfig::default());
    let mut ids = vec![0; n * IDS_PER_ROW];
    let mut floats = vec![0.0; n * FLOATS_PER_ROW];
    let mut field = vec![0.0; n * FIELD_PER_ROW];
    let mut foe_match = vec![0; n * FOE_MATCH_PER_ROW];
    let mut fainted = vec![0.0; n * FAINTED_PER_ROW];
    let mut guard = vec![false; n * GUARD_PER_ROW];
    let mut action_features = vec![0.0; n * ACTION_PER_ROW];
    let mut token_features = vec![0.0; n * TOKEN_FEATURES_PER_ROW];
    let mut out = Outputs { ids: &mut ids, floats: &mut floats, field: &mut field, foe_match: &mut foe_match,
        fainted: &mut fainted, guard: &mut guard, action_features: &mut action_features, token_features: &mut token_features };
    let mut step = env.reset()?;
    let mut t = Tally::default();
    let start = Instant::now();
    while start.elapsed() < budget {
        for &row in &rows {
            let o = env.observe(row / 2, row % 2)?;
            encoder.encode_row(row, &Observation { battle_id: o.battle_id, request: o.request.as_deref(), log: &o.log }, &mut out)?;
        }
        encoder.match_partners(&mut out);
        encoder.damage_features(&mut out);
        let obs = ObsRows { ids: out.ids, floats: out.floats, field: out.field,
            action_features: out.action_features, token_features: out.token_features };
        let mask0: Vec<bool> = step.mask0.iter().map(|&x| x != 0).collect();
        let first = smart.first(&obs, &rows, &mask0);
        let mask1: Vec<bool> = env.mask_slot1(&first)?.iter().map(|&x| x != 0).collect();
        let second = smart.second(&mask1);
        let actions: Vec<i32> = first.into_iter().zip(second).flat_map(|(a, b)| [a, b]).collect();
        t.sink += black_box(&actions).iter().map(|&x| x as usize).sum::<usize>();
        black_box(&out);
        step = env.step(&actions)?;
        t.items += n as u64;
        t.calls += 1;
    }
    Ok(t)
}

fn grid(roots: &[Root], ex: &mut GridExecutor, budget: Duration) -> Result<Tally, String> {
    let mut t = Tally::default();
    let start = Instant::now();
    while start.elapsed() < budget {
        for (k, r) in roots.iter().enumerate() {
            let text = r.game.is_text();
            let stop = if (t.calls / 2) % 2 == 0 { Stop::Boundary } else { Stop::Turn };
            let opts = Options { stop, requests: true, masks: k % 2 == 0, logs: text && k % 2 == 1, keep_leaves: false };
            let leaves = if t.calls % 4 == 3 {
                // Determinized worlds in one call: every world runs the first 25 cells with one common seed.
                let base = &r.actions[..100];
                let actions: Vec<i32> = (0..r.worlds.len()).flat_map(|_| base.iter().copied()).collect();
                let world: Vec<u32> = (0..r.worlds.len() as u32).flat_map(|w| [w; 25]).collect();
                let worlds: Vec<&Game> = r.worlds.iter().collect();
                ex.run(&worlds, &world, &actions, &Seeds::Fixed(vec![r.seeds[0]]), &opts)
            } else {
                ex.run(&[&r.game], &[], &r.actions, &Seeds::Fixed(r.seeds.clone()), &opts)
            };
            let leaves = leaves.map_err(|e| format!("GridExecutor failed on root {k}: {e}"))?;
            t.sink += leaves.iter().map(|l| l.requests[0].as_ref().map_or(0, String::len) + l.logs[0].len() + l.turn as usize).sum::<usize>();
            t.items += leaves.len() as u64;
            t.calls += 1;
        }
    }
    Ok(t)
}

fn primitives(roots: &[Root], budget: Duration) -> Result<Tally, String> {
    // One reusable worker per log mode (`restore_from` requires matching modes).
    let mut workers: Vec<Game> = Vec::new();
    for want in [false, true] {
        if let Some(r) = roots.iter().find(|r| r.game.is_text() == want) {
            workers.push(r.game.fork(false));
        }
    }
    let mut t = Tally::default();
    let start = Instant::now();
    while start.elapsed() < budget {
        for (k, r) in roots.iter().enumerate() {
            let worker = workers.iter_mut().find(|w| w.is_text() == r.game.is_text()).expect("one worker per log mode");
            for i in (k % 4..r.seeds.len()).step_by(4) {
                let codes = &r.actions[4 * i..4 * i + 4];
                t.sink += match (t.calls + i as u64) % 3 {
                    0 => cell(&mut r.game.fork(false), codes, r.seeds[i])?,
                    1 => {
                        worker.restore_from(&r.game)?;
                        cell(worker, codes, r.seeds[i])?
                    }
                    _ => {
                        worker.restore_from(&r.game)?;
                        for (index, set) in &r.swaps {
                            worker.replace_hidden_set(0, *index, set)?;
                        }
                        cell(worker, codes, r.seeds[i])?
                    }
                };
                t.items += 1;
            }
            t.calls += 1;
        }
    }
    Ok(t)
}

/// Run the workload for about `seconds` on `threads` threads with teams from `pool_paths` (the first 20,000
/// teams of each file). Returns a short per-phase report.
pub fn run(pool_paths: &[String], seconds: f64, threads: usize) -> Result<String, String> {
    if pool_paths.is_empty() {
        return Err("no team pool given".into());
    }
    if !(seconds >= 0.0 && seconds.is_finite()) {
        return Err(format!("seconds must be a non-negative number, got {seconds}"));
    }
    let pool = Arc::new(TeamPool::load(pool_paths, Some(20_000))
        .map_err(|e| format!("cannot load team pool {}: {e}", pool_paths.join(", ")))?);
    let share = |f: f64| Duration::from_secs_f64(seconds * f);
    let total = Instant::now();
    let mut report = format!("pgo workload: {} teams from {}, {seconds} s, {threads} thread(s)\n", pool.len(), pool_paths.join(", "));
    let mut line = |name: &str, unit: &str, t: &Tally, start: Instant| {
        let secs = start.elapsed().as_secs_f64();
        let _ = writeln!(report, "{name:>15}: {:>9} {unit} in {:>6} calls, {secs:.2} s ({:.0} {unit}/s)",
            t.items, t.calls, t.items as f64 / secs.max(1e-9));
        black_box(t.sink);
    };

    let start = Instant::now();
    line("selfplay-log", "battles", &selfplay(&pool, threads, true, share(0.25))?, start);
    let start = Instant::now();
    line("selfplay-nolog", "battles", &selfplay(&pool, threads, false, share(0.15))?, start);
    let start = Instant::now();
    line("selfplay-smart", "rows", &selfplay_smart(&pool, threads, share(0.15))?, start);

    let mut all = roots(&pool, 12, false, 2027)?;
    all.extend(roots(&pool, 12, true, 2028)?);
    let mut ex = GridExecutor::new(threads)?;
    let start = Instant::now();
    line("grid", "cells", &grid(&all, &mut ex, share(0.30))?, start);
    let start = Instant::now();
    line("primitives", "cells", &primitives(&all, share(0.15))?, start);
    let _ = writeln!(report, "pgo workload: done in {:.1} s", total.elapsed().as_secs_f64());
    Ok(report)
}
