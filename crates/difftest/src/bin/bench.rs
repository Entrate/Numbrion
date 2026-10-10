//! Engine throughput on recorded battles: replays each fixture's accepted choices with logging
//! off (training configuration) and reports battles/s, turns/s and decisions/s.
//!
//!   bench <fixtures.jsonl[.gz]> [--threads N] [--seconds S] [--textlog]

use std::io::BufRead;
use std::time::{Duration, Instant};

use difftest::fixture::Fixture;
use engine::log::{LogSink, NoLog, text::TextLog};

struct Battle {
    seed: [u16; 4],
    names: [String; 2],
    teams: [String; 2],
    choices: Vec<(usize, String)>,
    turns: u32,
}

fn load(path: &str) -> Vec<Battle> {
    let input = difftest::runner::open_input(path).expect("open fixtures");
    let mut out = Vec::new();
    for line in input.lines() {
        let line = line.expect("read");
        if line.trim().is_empty() {
            continue;
        }
        let fx: Fixture = serde_json::from_str(&line).expect("parse fixture");
        let mut choices = Vec::new();
        for step in &fx.steps {
            for c in &step.choices {
                let side = if c.side == "p1" { 0 } else { 1 };
                choices.push((side, c.input.clone()));
            }
        }
        out.push(Battle {
            seed: fx.battle_seed,
            names: [fx.players[0].name.clone(), fx.players[1].name.clone()],
            teams: [fx.teams[0].clone(), fx.teams[1].clone()],
            choices,
            turns: fx.end.turns,
        });
    }
    out
}

fn run_one<L: LogSink + Default>(b: &Battle) -> u32 {
    let mut battle = engine::Battle::from_players(
        b.seed,
        (&b.names[0], &b.teams[0]),
        (&b.names[1], &b.teams[1]),
        L::default(),
    )
    .expect("teams");
    battle.start().expect("start");
    for (side, input) in &b.choices {
        let _ = battle.choose(*side, input);
    }
    let outcome = battle.outcome().expect("battle did not end");
    assert_eq!(outcome.turns, b.turns, "turn count differs from fixture");
    outcome.turns
}

fn bench<L: LogSink + Default>(battles: &[Battle], threads: usize, seconds: f64) -> (f64, f64, f64) {
    let deadline = Duration::from_secs_f64(seconds);
    let decisions: u64 = battles.iter().map(|b| b.choices.len() as u64).sum();
    let results: Vec<(u64, u64, u64, Duration)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let start = Instant::now();
                    let (mut n, mut turns, mut dec) = (0u64, 0u64, 0u64);
                    while start.elapsed() < deadline {
                        for b in battles {
                            turns += run_one::<L>(b) as u64;
                            n += 1;
                        }
                        dec += decisions;
                    }
                    (n, turns, dec, start.elapsed())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let wall = results.iter().map(|r| r.3).max().unwrap().as_secs_f64();
    let n: u64 = results.iter().map(|r| r.0).sum();
    let t: u64 = results.iter().map(|r| r.1).sum();
    let d: u64 = results.iter().map(|r| r.2).sum();
    (n as f64 / wall, t as f64 / wall, d as f64 / wall)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().expect("usage: bench <fixtures> [--threads N] [--seconds S] [--textlog]");
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1));
    let threads: usize = get("--threads").map_or(1, |v| v.parse().unwrap());
    let seconds: f64 = get("--seconds").map_or(10.0, |v| v.parse().unwrap());
    let textlog = args.iter().any(|a| a == "--textlog");

    let battles = load(path);
    // Warm-up and correctness gate (every battle must end with the fixture's turn count).
    for b in &battles {
        run_one::<NoLog>(b);
    }
    let (bps, tps, dps) = if textlog {
        bench::<TextLog>(&battles, threads, seconds)
    } else {
        bench::<NoLog>(&battles, threads, seconds)
    };
    println!(
        "{} battles loaded | sink={} threads={} | {:.0} battles/s, {:.0} turns/s, {:.0} choose-calls/s ({:.1} battles/s/thread)",
        battles.len(),
        if textlog { "TextLog" } else { "NoLog" },
        threads,
        bps,
        tps,
        dps,
        bps / threads as f64
    );
}
