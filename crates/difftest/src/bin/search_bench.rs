//! Single-thread search primitives, including legacy construction/copy paths.
//! search_bench <fixtures> --op OP [--legacy] [--seconds 10]
//! OP: restore|clone|reset|construct|leaf, and for determinization:
//!   construct-defs  `from_team_defs` from pre-parsed teams (fresh scratch, no parsing)
//!   reset-defs      `reset_from_team_defs` with another battle's pre-parsed teams (reused scratch)
//!   replace         `clone_from(root)` + `replace_hidden_set` of one hidden p2 set
//!   replace-all     `clone_from(root)` + every hidden p2 set replaced (one sampled world)
use difftest::fixture::Fixture;
use engine::{
    Battle,
    ids::{MonId, SideId},
    log::NoLog,
    teams::{SetDef, TeamDef},
};
use std::{
    hint::black_box,
    io::BufRead,
    sync::Arc,
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
    let parsed: Vec<[Arc<TeamDef>; 2]> = fixtures
        .iter()
        .map(|f| [0, 1].map(|s| Arc::new(TeamDef::unpack(&f.teams[s]).unwrap())))
        .collect();
    let names: Vec<[String; 2]> = fixtures.iter().map(|f| [f.name(0).to_owned(), f.name(1).to_owned()]).collect();
    // Viewer p1; replacement sets come from the next battle's p2 team, in order.
    let hidden: Vec<Vec<(MonId, SetDef)>> = roots
        .iter()
        .enumerate()
        .map(|(i, root)| {
            let donor = &parsed[(i + 1) % parsed.len()][1];
            let mask = root.hidden_mons(SideId(0));
            let mut trial = root.clone();
            // Skip donor sets the API refuses (BattleStart species after start).
            (0..12)
                .filter(|&m| mask & (1 << m) != 0)
                .map(|m| (MonId(m), donor.sets[m as usize % 6].clone()))
                .filter(|(m, set)| trial.replace_hidden_set(SideId(0), *m, set).is_ok())
                .collect()
        })
        .collect();
    let mut next = 0;
    let start = Instant::now();
    let mut count = 0u64;
    while start.elapsed() < Duration::from_secs_f64(seconds) {
        for _ in 0..32 {
            for (i, ((f, root), worker)) in fixtures.iter().zip(&roots).zip(&mut workers).enumerate() {
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
                    "construct-defs" => {
                        let [p1, p2] = parsed[next % parsed.len()].clone();
                        let names = names[next % names.len()].clone();
                        black_box(Battle::from_team_defs(f.battle_seed, p1, p2, names, NoLog).unwrap());
                        next += 1;
                    }
                    "reset-defs" => {
                        // A different battle's teams each time, as when sampling worlds.
                        let [p1, p2] = parsed[next % parsed.len()].clone();
                        worker.reset_from_team_defs(f.battle_seed, p1, p2).unwrap();
                        black_box(&*worker);
                        next += 1;
                    }
                    "replace" | "replace-all" => {
                        worker.clone_from(root);
                        let sets = &hidden[i];
                        let n = if op == "replace" { sets.len().min(1) } else { sets.len() };
                        for (mon, set) in &sets[..n] {
                            worker.replace_hidden_set(SideId(0), *mon, set).unwrap();
                        }
                        black_box(&*worker);
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
    if op.starts_with("replace") {
        let swaps: usize = hidden.iter().map(Vec::len).sum();
        println!("hidden p2 sets per root: {:.2}", swaps as f64 / hidden.len() as f64);
    }
    println!(
        "{op} {}: {count} operations in {elapsed:.3}s = {:.3} us/op",
        if legacy { "legacy" } else { "worker" },
        elapsed * 1e6 / count as f64
    );
}
