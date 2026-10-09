//! Orchestration: stream a fixture file through a [`Sim`] on a thread pool and aggregate a [`Report`].

use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::replay::{ReplayOpts, install_panic_hook, replay_line};
use crate::report::{Aggregator, Report};
use crate::runner::{Selection, for_each_line};
use crate::sim::Sim;

pub struct ReplayConfig {
    pub input: String,
    pub sim_name: String,
    pub selection: Selection,
    pub jobs: usize,
    pub stop_on_first: bool,
    pub opts: ReplayOpts,
    /// Print a progress line to stderr (only if stderr is a terminal).
    pub progress: bool,
    /// Abort the process (exit status 3) if a single battle runs longer than this: an engine stuck in
    /// an infinite loop would otherwise hang the whole run silently. `None` disables the watchdog.
    pub battle_timeout: Option<Duration>,
    /// Command line printed by the watchdog so the hung battle can be re-run alone.
    pub repro_cmd: String,
}

/// Battles currently being replayed (position -> start time), watched by the watchdog thread.
#[derive(Default)]
struct InFlight(Mutex<HashMap<usize, Instant>>);

fn watchdog(in_flight: &InFlight, done: &AtomicBool, limit: Duration, repro_cmd: &str) {
    // Sleep in short slices so the run can finish promptly; look at the battles every ~250 ms.
    let mut slices = 0u32;
    while !done.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(5));
        slices += 1;
        if !slices.is_multiple_of(50) {
            continue;
        }
        let map = in_flight.0.lock().unwrap();
        if let Some((pos, t)) = map.iter().find(|(_, t)| t.elapsed() > limit) {
            eprintln!(
                "difftest: battle {pos} has been running for {:.0}s (limit {}s): the Sim is probably stuck in a loop.\n\
                 difftest: aborting; re-run it alone with: {repro_cmd} --battle {pos} --jobs 1",
                t.elapsed().as_secs_f64(),
                limit.as_secs()
            );
            std::process::exit(3);
        }
    }
}

pub fn run_replay<S: Sim>(cfg: &ReplayConfig) -> io::Result<Report> {
    if cfg.opts.catch_panics {
        install_panic_hook();
    }
    let stop = AtomicBool::new(false);
    let done_flag = AtomicBool::new(false);
    let in_flight = InFlight::default();
    let mut agg = Aggregator::default();
    let t0 = Instant::now();
    let show_progress = cfg.progress && io::stderr().is_terminal();
    let mut last_draw = Instant::now();
    let mut done = 0usize;
    let result = std::thread::scope(|scope| {
        if let Some(limit) = cfg.battle_timeout {
            let (in_flight, done_flag) = (&in_flight, &done_flag);
            scope.spawn(move || watchdog(in_flight, done_flag, limit, &cfg.repro_cmd));
        }
        let r = for_each_line(
            &cfg.input,
            &cfg.selection,
            cfg.jobs,
            &stop,
            |pos, line| {
                if cfg.battle_timeout.is_some() {
                    in_flight.0.lock().unwrap().insert(pos, Instant::now());
                }
                let r = replay_line::<S>(pos, line, &cfg.opts);
                if cfg.battle_timeout.is_some() {
                    in_flight.0.lock().unwrap().remove(&pos);
                }
                r
            },
            |_, result| {
                done += 1;
                if !result.passed() && cfg.stop_on_first {
                    stop.store(true, Ordering::Relaxed);
                }
                agg.add(result);
                if show_progress && last_draw.elapsed() > Duration::from_millis(200) {
                    last_draw = Instant::now();
                    eprint!("\r  {done} battles, {} failed   ", agg.failed());
                    let _ = io::stderr().flush();
                }
            },
        );
        done_flag.store(true, Ordering::Relaxed);
        r
    });
    result?;
    if show_progress {
        eprint!("\r{:40}\r", "");
    }
    Ok(agg.finish(&cfg.input, &cfg.sim_name, cfg.jobs, t0.elapsed().as_secs_f64()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockSim;
    use crate::selftest::synthetic_fixture;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::fs::File;

    fn cfg(path: &str) -> ReplayConfig {
        ReplayConfig {
            input: path.to_string(),
            sim_name: "mock".into(),
            selection: Selection::default(),
            jobs: 3,
            stop_on_first: false,
            opts: ReplayOpts::default(),
            progress: false,
            battle_timeout: Some(Duration::from_secs(60)),
            repro_cmd: "difftest replay x".into(),
        }
    }

    #[test]
    fn replays_plain_and_gzip_files() {
        let dir = std::env::temp_dir().join(format!("difftest-driver-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let line = serde_json::to_string(&synthetic_fixture()).unwrap();
        let (plain, gz) = (dir.join("f.jsonl"), dir.join("f.jsonl.gz"));
        let body: String = (0..30).map(|_| format!("{line}\n")).collect();
        std::fs::write(&plain, &body).unwrap();
        let mut enc = GzEncoder::new(File::create(&gz).unwrap(), Compression::fast());
        enc.write_all(body.as_bytes()).unwrap();
        enc.finish().unwrap();
        for path in [&plain, &gz] {
            let rep = run_replay::<MockSim>(&cfg(path.to_str().unwrap())).unwrap();
            assert_eq!((rep.summary.total, rep.summary.passed, rep.summary.failed), (30, 30, 0), "{path:?}");
            assert_eq!(rep.summary.steps_matched, 90);
        }
        // A broken line is a `fixture` failure with its position, the rest still runs.
        let broken = format!("{line}\n{{oops\n{line}\n");
        std::fs::write(&plain, broken).unwrap();
        let rep = run_replay::<MockSim>(&cfg(plain.to_str().unwrap())).unwrap();
        assert_eq!((rep.summary.total, rep.summary.passed, rep.summary.failed), (3, 2, 1));
        assert_eq!(rep.failures[0].pos, 1);
        assert_eq!(rep.signatures[0].signature, "fixture: unusable line");
        std::fs::remove_dir_all(&dir).ok();
    }
}
