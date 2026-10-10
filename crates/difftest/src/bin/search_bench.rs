//! Single-thread search primitives, including legacy construction/copy paths.
//! search_bench <fixtures> --op restore|clone|reset|construct|leaf [--legacy] [--seconds 10]
use difftest::fixture::Fixture;
use engine::{Battle, log::NoLog};
use std::{
    hint::black_box,
    io::BufRead,
    time::{Duration, Instant},
};

fn construct(f: &Fixture) -> Battle {
    Battle::from_players(f.battle_seed, (f.name(0), &f.teams[0]), (f.name(1), &f.teams[1]), NoLog).unwrap()
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let get = |key: &str| args.iter().position(|s| s == key).and_then(|i| args.get(i + 1));
    let op = get("--op").map(String::as_str).unwrap_or("restore");
    let legacy = args.iter().any(|s| s == "--legacy");
    let seconds = get("--seconds").map_or(10.0, |s| s.parse::<f64>().unwrap());
    // Keep a small diverse working set: roots/scratch setup is outside timing.
    let fixtures: Vec<Fixture> = difftest::runner::open_input(&args[0])
        .unwrap()
        .lines()
        .take(32)
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let mut roots: Vec<_> = fixtures.iter().map(construct).collect();
    for root in &mut roots {
        root.start().unwrap();
    }
    let mut workers: Vec<_> = roots.iter().map(Clone::clone).collect();
    for (worker, root) in workers.iter_mut().zip(&roots) {
        worker.restore_from(root);
    }
    let start = Instant::now();
    let mut count = 0u64;
    while start.elapsed() < Duration::from_secs_f64(seconds) {
        for _ in 0..32 {
            for ((f, root), worker) in fixtures.iter().zip(&roots).zip(&mut workers) {
                match op {
                    "restore" => {
                        if legacy {
                            worker.state = root.state;
                        } else {
                            worker.restore_from(root);
                        }
                        black_box(&*worker);
                    }
                    "clone" => {
                        let copy = if legacy {
                            let mut b = construct(f);
                            b.state = root.state;
                            b
                        } else {
                            root.clone()
                        };
                        black_box(copy);
                    }
                    "reset" => {
                        if legacy {
                            *worker = construct(f);
                        } else {
                            worker.reset_seed(f.battle_seed).unwrap();
                        }
                        black_box(&*worker);
                    }
                    "construct" => {
                        black_box(construct(f));
                    }
                    "leaf" => {
                        if legacy {
                            worker.state = root.state;
                        } else {
                            worker.restore_from(root);
                        }
                        for c in &f.steps[0].choices {
                            let _ = worker.choose(if c.side == "p1" { 0 } else { 1 }, &c.input);
                        }
                        black_box(&*worker);
                    }
                    _ => panic!("unknown operation"),
                }
                count += 1;
            }
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "{op} {}: {count} operations in {elapsed:.3}s = {:.3} us/op",
        if legacy { "legacy" } else { "worker" },
        elapsed * 1e6 / count as f64
    );
}
