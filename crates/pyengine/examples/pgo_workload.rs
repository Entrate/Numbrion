//! The PGO profile-collection workload (`_numbrion::pgo`, see there for the phases) from cargo, without
//! Python: a smoke test and timing run for the workload that `tools/pgo.sh` / `tools/pgo.ps1` run from an
//! instrumented wheel (`_numbrion._pgo_workload`). A profile collected from this binary fits binaries
//! built the same way, not the wheel (see `src/pgo.rs`).
//!
//! `cargo run --release -p pyengine --example pgo_workload -- [--seconds 10] [--threads 1] [--pool path]...`
//!
//! Teams come from `--pool` (repeatable; e.g. the training pools data/teams/pool-s*-200k.txt.gz) or the
//! bundled test pool.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |key: &str| args.iter().position(|s| s == key).map(|i| args.get(i + 1).cloned().unwrap_or_default());
    let num = |key: &str, default: f64| match get(key) {
        None => default,
        Some(s) => s.parse().unwrap_or_else(|_| {
            eprintln!("pgo_workload: {key} takes a number, got {s:?}");
            std::process::exit(2);
        }),
    };
    let seconds = num("--seconds", 10.0);
    let threads = num("--threads", 1.0) as usize;
    let mut pools: Vec<String> = args.windows(2).filter(|w| w[0] == "--pool").map(|w| w[1].clone()).collect();
    if pools.is_empty() {
        pools.push(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/pool-400.txt.gz").into());
    }
    match _numbrion::pgo::run(&pools, seconds, threads) {
        Ok(report) => print!("{report}"),
        Err(e) => {
            eprintln!("pgo_workload: {e}");
            std::process::exit(1);
        }
    }
}
