//! Grid-search micro-benchmark: 10 x 10 cells (10 joint actions per side) from mid-battle roots.
//!
//! `cargo run --release -p pyengine --example grid_bench -- [--seconds 1] [--rounds 4] [--roots 32] [--threads 1,3]
//! [--log] [--pool path]`
//!
//! Modes: `executor/N` is `GridExecutor` with N threads; `restore` is the same per-cell work on one reusable
//! worker without the pool (`Game::restore_from` + reseed + both choices + leaf requests); `clone` makes a
//! fresh copy per cell (`Game::duplicate`) instead. Every cell is reseeded and returns both requests (and
//! the player-view lines with `--log`).

use _numbrion::{
    env::{SamplerParams, sample_from},
    game::Game,
    pool::{SplitMix, TeamPool},
    search::{GridExecutor, Options, Seeds},
};
use engine::state::choices::SlotChoice;
use std::time::Instant;

struct Root {
    game: Game,
    actions: Vec<i32>,
    /// The engine choices of every cell, precomputed for the sequential modes.
    choices: Vec<[[SlotChoice; 2]; 2]>,
    seeds: Vec<[u16; 4]>,
}

/// Up to ten distinct legal pairs of one side (repeats if it has fewer).
fn pairs(g: &Game, side: usize, rng: &mut SplitMix) -> Vec<[i32; 2]> {
    let a = g.side_actions(side);
    let p = SamplerParams { switch_prob: 0.2, tera_prob: 0.2 };
    let mut out: Vec<[i32; 2]> = Vec::new();
    for _ in 0..200 {
        let a0 = sample_from(a.mask0(), rng, &p);
        let pair = [a0 as i32, sample_from(a.mask1(a0), rng, &p) as i32];
        if !out.contains(&pair) {
            out.push(pair);
        }
        if out.len() == 10 {
            break;
        }
    }
    let distinct = out.len();
    for i in distinct..10 {
        out.push(out[i % distinct]);
    }
    out
}

fn roots(pool: &TeamPool, count: usize, log: bool) -> Vec<Root> {
    let mut rng = SplitMix::new(2026);
    let p = SamplerParams::default();
    let mut out = Vec::new();
    while out.len() < count {
        let (i, j) = (rng.below(pool.len()), rng.below(pool.len()));
        let Ok(mut g) = Game::new(rng.battle_seed(), pool.get(i).clone(), pool.get(j).clone(),
            ["Alice".into(), "Bob".into()], log, false) else { continue };
        g.start().unwrap();
        let target = 3 + rng.below(8) as u32;
        while !g.ended() && g.turn() < target {
            let mut acts = [-1i32; 4];
            // Both masks before either side chooses: the second choice may run the turn.
            let sides = [g.side_actions(0), g.side_actions(1)];
            let party = [g.party(0), g.party(1)];
            for (s, a) in sides.iter().enumerate() {
                if a.needs_action() {
                    let a0 = sample_from(a.mask0(), &mut rng, &p);
                    acts[2 * s] = a0 as i32;
                    acts[2 * s + 1] = sample_from(a.mask1(a0), &mut rng, &p) as i32;
                    let ch = a.choices(a0, acts[2 * s + 1] as usize, &party[s]).unwrap();
                    g.choose_typed(s, &ch).unwrap();
                }
            }
        }
        // A mid-battle turn boundary where both sides choose.
        if g.ended() || !(0..2).all(|s| g.side_actions(s).req == _numbrion::mask::Req::Move) {
            continue;
        }
        let (p1, p2) = (pairs(&g, 0, &mut rng), pairs(&g, 1, &mut rng));
        let actions: Vec<i32> = p1.iter().flat_map(|a| p2.iter().flat_map(move |b| [a[0], a[1], b[0], b[1]])).collect();
        let acts = [g.side_actions(0), g.side_actions(1)];
        let choices = actions.chunks(4).map(|c| [0, 1].map(|s| acts[s].choices(c[2 * s] as usize, c[2 * s + 1] as usize,
            &g.party(s)).unwrap())).collect();
        let seeds = (0..100).map(|_| rng.battle_seed()).collect();
        g.drain_player(0);
        g.drain_player(1);
        out.push(Root { game: g, actions, choices, seeds });
    }
    out
}

/// One cell the sequential way on `g` (already a copy of the root).
fn cell(g: &mut Game, choices: &[[SlotChoice; 2]; 2], seed: [u16; 4], log: bool) -> usize {
    g.reseed(seed);
    for (s, ch) in choices.iter().enumerate() {
        g.choose_typed(s, ch).unwrap();
    }
    let mut n = g.request_json(0).map_or(0, |r| r.len()) + g.request_json(1).map_or(0, |r| r.len());
    if log {
        n += g.drain_player(0).len() + g.drain_player(1).len();
    }
    n
}

/// One timed pass over every root (100 cells each); returns the cells run.
fn pass(mode: &str, roots: &[Root], ex: &mut [GridExecutor], worker: &mut Game, opts: &Options, sink: &mut usize) -> u64 {
    let mut cells = 0;
    for r in roots {
        if let Some(t) = mode.strip_prefix("executor/") {
            let ex = ex.iter_mut().find(|e| e.threads().to_string() == t).unwrap();
            let leaves = ex.run(&[&r.game], &[], &r.actions, &Seeds::Fixed(r.seeds.clone()), opts).unwrap();
            *sink += leaves.iter().map(|l| l.requests[0].as_ref().map_or(0, String::len) + l.logs[0].len()).sum::<usize>();
            cells += leaves.len() as u64;
            continue;
        }
        for i in 0..100 {
            if mode == "restore" {
                worker.restore_from(&r.game).unwrap();
                *sink += cell(worker, &r.choices[i], r.seeds[i], opts.logs);
            } else {
                let mut g = r.game.fork(false);
                *sink += cell(&mut g, &r.choices[i], r.seeds[i], opts.logs);
            }
            cells += 1;
        }
    }
    cells
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |key: &str| args.iter().position(|s| s == key).and_then(|i| args.get(i + 1)).cloned();
    let seconds: f64 = get("--seconds").map_or(1.0, |s| s.parse().unwrap());
    let rounds: usize = get("--rounds").map_or(4, |s| s.parse().unwrap());
    let count: usize = get("--roots").map_or(32, |s| s.parse().unwrap());
    let threads: Vec<usize> = get("--threads").map_or(vec![1, 3], |s| s.split(',').map(|t| t.parse().unwrap()).collect());
    let log = args.iter().any(|s| s == "--log");
    let path = get("--pool").unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/pool-400.txt.gz").into());
    let pool = TeamPool::load(&[path], None).unwrap();
    let roots = roots(&pool, count, log);
    let opts = Options { logs: log, ..Options::default() };
    let mut modes: Vec<String> = threads.iter().map(|t| format!("executor/{t}")).collect();
    modes.extend(["restore".to_string(), "clone".to_string()]);
    println!("{} roots x 100 cells, log={log}, {rounds} rounds x {seconds} s per mode, order alternating", roots.len());
    let mut ex: Vec<GridExecutor> = threads.iter().map(|&t| GridExecutor::new(t).unwrap()).collect();
    let mut worker = roots[0].game.fork(false);
    let mut sink = 0usize;
    let mut totals = vec![(0u64, 0f64, Vec::new()); modes.len()];
    for round in 0..rounds {
        let order: Vec<usize> = if round % 2 == 0 { (0..modes.len()).collect() } else { (0..modes.len()).rev().collect() };
        for m in order {
            let (start, mut cells) = (Instant::now(), 0u64);
            while start.elapsed().as_secs_f64() < seconds {
                cells += pass(&modes[m], &roots, &mut ex, &mut worker, &opts, &mut sink);
            }
            let secs = start.elapsed().as_secs_f64();
            totals[m].0 += cells;
            totals[m].1 += secs;
            totals[m].2.push(secs * 1e6 / cells as f64);
        }
    }
    for (m, (cells, secs, per)) in totals.iter().enumerate() {
        let rounds: Vec<String> = per.iter().map(|x| format!("{x:.1}")).collect();
        println!("{:>12}: {:>8.0} cells/s  {:>7.2} us/cell  (rounds: {} us/cell)", modes[m], *cells as f64 / secs,
            secs * 1e6 / *cells as f64, rounds.join(" "));
    }
    std::hint::black_box(sink);
}
