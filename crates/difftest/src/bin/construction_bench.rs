//! Isolated Battle::from_players throughput on recorded packed teams.
//! Usage: construction_bench <fixtures.jsonl[.gz]> [--seconds S]
use std::hint::black_box;
use std::io::BufRead;
use std::time::{Duration, Instant};

use difftest::fixture::Fixture;
use engine::log::NoLog;

struct Input {
    seed: [u16; 4],
    names: [String; 2],
    teams: [String; 2],
}

fn load(path: &str) -> Vec<Input> {
    let input = difftest::runner::open_input(path).expect("open fixtures");
    input
        .lines()
        .filter_map(|line| {
            let line = line.expect("read");
            if line.trim().is_empty() {
                return None;
            }
            let fx: Fixture = serde_json::from_str(&line).expect("parse fixture");
            Some(Input {
                seed: fx.battle_seed,
                names: [fx.players[0].name.clone(), fx.players[1].name.clone()],
                teams: [fx.teams[0].clone(), fx.teams[1].clone()],
            })
        })
        .collect()
}

fn construct(input: &Input) {
    let battle = engine::Battle::from_players(
        input.seed,
        (&input.names[0], &input.teams[0]),
        (&input.names[1], &input.teams[1]),
        NoLog,
    )
    .expect("teams");
    black_box(battle);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().expect("usage: construction_bench <fixtures> [--seconds S]");
    let seconds = args
        .iter()
        .position(|arg| arg == "--seconds")
        .and_then(|i| args.get(i + 1))
        .map_or(10.0, |s| s.parse::<f64>().expect("seconds"));
    let inputs = load(path);
    assert!(!inputs.is_empty());
    for input in &inputs {
        construct(input);
    }
    let start = Instant::now();
    let mut count = 0usize;
    let deadline = Duration::from_secs_f64(seconds);
    while start.elapsed() < deadline {
        for input in &inputs {
            construct(input);
            count += 1;
        }
    }
    let elapsed = start.elapsed();
    println!(
        "{} teams | {} constructions in {:.3}s | {:.1} us/construction",
        inputs.len(),
        count,
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1_000_000.0 / count as f64
    );
}
