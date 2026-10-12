//! SearchGrid without Python: per-cell cost of the fused path (engine + leaf encoding) against the engine-only
//! grid, plus a one-thread breakdown of the encoding steps on the same leaves.
//!
//!     cargo run --release -p pyengine --example search_grid_bench -- [ROOTS] [THREADS...]
//!
//! Roots are turn 3-10 boundaries of random self-play on the test pool where both sides choose; each gets 8 x 8
//! joint actions and one seed per cell.
use _numbrion::{
    env::{SamplerParams, sample_from},
    game::Game,
    pool::{SplitMix, TeamPool},
    search::{GridExecutor, Options, Seeds},
    search_grid::{LeafOptions, SearchGrid},
};
use encoder::{
    Dex,
    encode::{self, Encoder, Observation, Outputs, RowOutputs, Statics},
    layout::*,
    tracker::Tracker,
};
use std::{hint::black_box, path::Path, time::Instant};

struct Root {
    game: Game,
    trackers: [Tracker; 2],
    actions: Vec<i32>,
    seeds: Vec<[u16; 4]>,
}

fn observe(enc: &mut Encoder, g: &mut Game, out: &mut Outputs<'_>) {
    for s in 0..2 {
        let lines = g.drain_player(s);
        let request = g.request_json(s);
        enc.encode_row(s, &Observation { battle_id: 0, request: request.as_deref(), log: &lines }, out).unwrap();
    }
}

fn pairs(g: &Game, side: usize, rng: &mut SplitMix, k: usize) -> Vec<[i32; 2]> {
    let a = g.side_actions(side);
    let p = SamplerParams::default();
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
    (0..k).map(|i| out[i % out.len()]).collect()
}

fn main() {
    let args: Vec<usize> = std::env::args().skip(1).map(|a| a.parse().expect("numbers")).collect();
    let n_roots = args.first().copied().unwrap_or(16);
    let threads: Vec<usize> = if args.len() > 1 { args[1..].to_vec() } else { vec![1, 2, 4] };
    let pool = TeamPool::load(&[concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/pool-400.txt.gz")], None).unwrap();
    let dex = Dex::shared(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json")).unwrap();
    let mut rng = SplitMix::new(2026);
    let p = SamplerParams::default();
    let mut bufs = (vec![0i64; 2 * IDS_PER], vec![0f32; 2 * FL_PER], vec![0f32; 2 * N_FIELD_FLOATS], vec![0i64; 12],
        vec![0f32; 4], vec![false; 32], vec![0f32; 2 * encode::ACTION_PER_ROW], vec![0f32; 2 * encode::TOKEN_FEATURES_PER_ROW]);
    let mut roots = Vec::new();
    while roots.len() < n_roots {
        let (i, j) = (rng.below(pool.len()), rng.below(pool.len()));
        let mut g = Game::new(rng.battle_seed(), pool.get(i).clone(), pool.get(j).clone(), ["A".into(), "B".into()], true, false).unwrap();
        g.start().unwrap();
        let mut enc = Encoder::new(dex.clone(), 2);
        let target = 3 + rng.below(8) as u32;
        let mut out = Outputs { ids: &mut bufs.0, floats: &mut bufs.1, field: &mut bufs.2, foe_match: &mut bufs.3,
            fainted: &mut bufs.4, guard: &mut bufs.5, action_features: &mut bufs.6, token_features: &mut bufs.7 };
        observe(&mut enc, &mut g, &mut out);
        while !g.ended() && g.turn() < target {
            // Both sides answer the same boundary (never a side's choice at the next one: a half-chosen root).
            let acts = [g.side_actions(0), g.side_actions(1)];
            for (s, a) in acts.iter().enumerate() {
                if a.needs_action() {
                    let a0 = sample_from(a.mask0(), &mut rng, &p);
                    let ch = a.choices(a0, sample_from(a.mask1(a0), &mut rng, &p), &g.party(s)).unwrap();
                    g.choose_typed(s, &ch).unwrap();
                }
            }
            observe(&mut enc, &mut g, &mut out);
        }
        if g.ended() || !g.side_actions(0).needs_action() || !g.side_actions(1).needs_action() || g.side_actions(0).req.code() != 1 {
            continue;
        }
        let (p0, p1) = (pairs(&g, 0, &mut rng, 8), pairs(&g, 1, &mut rng, 8));
        let actions: Vec<i32> = p0.iter().flat_map(|a| p1.iter().flat_map(move |b| [a[0], a[1], b[0], b[1]])).collect();
        let seeds = (0..actions.len() / 4).map(|_| rng.battle_seed()).collect();
        roots.push(Root { trackers: [enc.tracker(0).clone(), enc.tracker(1).clone()], game: g, actions, seeds });
    }
    let cells: usize = roots.iter().map(|r| r.actions.len() / 4).sum();
    println!("{} roots, {cells} cells", roots.len());
    if let Ok(secs) = std::env::var("SG_PROFILE_SECONDS") {
        // Only the fused path on one thread, for a profiler.
        let mut sg = SearchGrid::new(dex.clone(), 1).unwrap();
        let start = Instant::now();
        let mut n = 0;
        while start.elapsed().as_secs_f64() < secs.parse::<f64>().unwrap() {
            for r in &roots {
                let pair = [&r.trackers[0], &r.trackers[1]];
                black_box(sg.run(&[&r.game], &[], &r.actions, &Seeds::Fixed(r.seeds.clone()), &[pair], 0, &LeafOptions::default()).unwrap());
                n += r.actions.len() / 4;
            }
        }
        println!("fused {:.1} us/cell", start.elapsed().as_secs_f64() * 1e6 / n as f64);
        return;
    }
    let reps = 3;
    for &t in &threads {
        let mut ex = GridExecutor::new(t).unwrap();
        let mut sg = SearchGrid::new(dex.clone(), t).unwrap();
        let opts = Options { requests: true, logs: true, ..Options::default() };
        let mut best = [f64::MAX; 3];
        for _ in 0..reps {
            let start = Instant::now();
            for r in &roots {
                black_box(ex.run(&[&r.game], &[], &r.actions, &Seeds::Fixed(r.seeds.clone()), &Options::default()).unwrap());
            }
            best[0] = best[0].min(start.elapsed().as_secs_f64());
            let start = Instant::now();
            for r in &roots {
                black_box(ex.run(&[&r.game], &[], &r.actions, &Seeds::Fixed(r.seeds.clone()), &opts).unwrap());
            }
            best[1] = best[1].min(start.elapsed().as_secs_f64());
            let start = Instant::now();
            for r in &roots {
                let pair = [&r.trackers[0], &r.trackers[1]];
                black_box(sg.run(&[&r.game], &[], &r.actions, &Seeds::Fixed(r.seeds.clone()), &[pair], 0, &LeafOptions::default()).unwrap());
            }
            best[2] = best[2].min(start.elapsed().as_secs_f64());
        }
        let us = |s: f64| s * 1e6 / cells as f64;
        println!("threads {t}: grid (requests only) {:.1} us/cell, grid + logs {:.1}, fused {:.1}", us(best[0]), us(best[1]), us(best[2]));
    }

    // One-thread breakdown on the leaves of the grid (logs and requests from GridExecutor).
    let mut ex = GridExecutor::new(1).unwrap();
    let opts = Options { requests: true, logs: true, ..Options::default() };
    let mut leaves = Vec::new();
    for r in &roots {
        for l in ex.run(&[&r.game], &[], &r.actions, &Seeds::Fixed(r.seeds.clone()), &opts).unwrap() {
            leaves.push((r, l));
        }
    }
    let mut cache = Statics::default();
    let damage = encoder::damage::DamageFeatures::new(&dex);
    let mut tr = Tracker::new(0);
    let mut times = [0.0f64; 7];
    let (mut ids, mut floats, mut field, mut tf, mut af) =
        (vec![0i64; IDS_PER], vec![0f32; FL_PER], vec![0f32; N_FIELD_FLOATS], vec![0f32; encode::TOKEN_FEATURES_PER_ROW], vec![0f32; encode::ACTION_PER_ROW]);
    for _ in 0..reps {
        for (r, l) in &leaves {
            for s in 0..2 {
                let Some(req) = &l.requests[s] else { continue };
                let t0 = Instant::now();
                tr.clone_from(&r.trackers[s]);
                let t1 = Instant::now();
                tr.feed(&l.logs[s]);
                let t2 = Instant::now();
                let json = encode::parse_request(Some(req)).unwrap();
                let t3 = Instant::now();
                let out = RowOutputs { ids: &mut ids, floats: &mut floats, field: Some(&mut field), fainted: None, guard: None };
                encode::encode_tracker(&dex, &mut cache, &tr, &json, out, None).unwrap();
                let t4 = Instant::now();
                damage.token_row(&ids, &floats, &field, &mut tf);
                let t5 = Instant::now();
                damage.row(&ids, &floats, &field, &mut af, &mut tf);
                let t6 = Instant::now();
                let out = RowOutputs { ids: &mut ids[..6 * N_IDS], floats: &mut floats[..6 * N_POKEMON_FLOATS], field: None, fainted: None, guard: None };
                encode::encode_own_tokens(&dex, &mut cache, &tr, &json, out).unwrap();
                let t7 = Instant::now();
                for (k, (a, b)) in [(t0, t1), (t1, t2), (t2, t3), (t3, t4), (t4, t5), (t5, t6), (t6, t7)].iter().enumerate() {
                    times[k] += (*b - *a).as_secs_f64();
                }
            }
        }
    }
    let rows = (reps * leaves.len() * 2) as f64;
    let names = ["tracker clone", "feed", "parse request", "encode row", "token features", "token+action features", "own tokens"];
    for (n, t) in names.iter().zip(times) {
        println!("  {n:24} {:6.2} us/row", t * 1e6 / rows);
    }
    let mut text = String::new();
    let start = Instant::now();
    for _ in 0..100 {
        for r in &roots {
            for s in 0..2 {
                text.clear();
                black_box(r.game.write_request_json(s, &mut text));
            }
        }
    }
    println!("  {:24} {:6.2} us/row", "write request json", start.elapsed().as_secs_f64() * 1e6 / (200 * roots.len()) as f64);
    let start = Instant::now();
    for _ in 0..100 {
        for r in &roots {
            for s in 0..2 {
                black_box(r.game.request_tree(s));
            }
        }
    }
    println!("  {:24} {:6.2} us/row (replaces write + parse)", "request tree", start.elapsed().as_secs_f64() * 1e6 / (200 * roots.len()) as f64);
    let lines: usize = leaves.iter().map(|(_, l)| l.logs[0].len() + l.logs[1].len()).sum();
    let bytes: usize = leaves.iter().map(|(_, l)| l.requests.iter().flatten().map(|r| r.len()).sum::<usize>()).sum();
    println!("  {:.1} lines per side, {:.0} request bytes per side", lines as f64 / (2 * leaves.len()) as f64, bytes as f64 / (2 * leaves.len()) as f64);
}

const IDS_PER: usize = N_TOKENS * N_IDS;
const FL_PER: usize = N_TOKENS * N_POKEMON_FLOATS;
