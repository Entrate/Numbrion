//! `difftest`: replay Showdown oracle fixtures through an engine and report divergences.

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use difftest::driver::{ReplayConfig, run_replay};
use difftest::fixture::Fixture;
use difftest::mock::{Corruption, MockSim};
use difftest::replay::ReplayOpts;
use difftest::report::{RenderOpts, Report, render_human};
use difftest::runner::{Selection, default_jobs, for_each_line};
use difftest::selftest;
use difftest::stats::Stats;

const USAGE: &str = "\
difftest: differential-test harness for numbrion (replays Showdown oracle fixtures)

USAGE
  difftest replay <fixtures.jsonl|.jsonl.gz|-> [options]   replay fixtures through a simulator
  difftest stats  <fixtures.jsonl|.jsonl.gz|-> [options]   fixture statistics
  difftest selftest [fixtures.jsonl] [--limit N]           inject faults into the mock, check the harness catches them
  difftest help

REPLAY OPTIONS
  --sim engine|mock      simulator to test (default: engine if built in, else mock)
  --limit N              stop after N battles          --skip N   skip the first N battles
  --battle SPEC          only these battles, 0-based positions in the input: 3  or  3,7,10-20
  --stop-on-first        stop at the first failing battle (with --jobs > 1 a few more may be in flight)
  --jobs K               worker threads (default: all cores)
  --json                 print one machine-readable JSON document (summary, signatures, every failure)
  --show N               failing battles printed in detail (default 5)     --sig TEXT  only show failures whose signature contains TEXT
  --top N                rows of the signature table (default 40)           --context N matching log lines of context (default 15)
  --strict-order         request key-order differences fail the battle (default: warning)
  --keep-indices         signatures keep array indices (moves[2] instead of moves[])
  --full                 include whole expected/actual request texts in request divergences
  --no-catch-panics      let Sim panics abort the run (default: a panic is a `panic` divergence)
  --failed-out FILE      write the failing battle positions (comma separated, usable as --battle \"$(cat FILE)\")
  --corrupt SPEC         (mock only) inject a fault into every battle; SPECs below
  --battle-timeout SECS  abort (exit 3) if one battle runs longer than this: a hung engine (default 120, 0 = off)
  --quiet                no progress line

STATS OPTIONS
  --limit N --skip N --battle SPEC --jobs K --top N (log kinds, default 30) --json

EXIT STATUS  0 all battles passed, 1 some failed, 2 usage or I/O error

CORRUPT SPECS
";

fn usage() -> String {
    format!("{USAGE}{}\n", Corruption::SYNTAX)
}

struct Parsed {
    positional: Vec<String>,
    values: HashMap<String, String>,
    flags: HashSet<String>,
}

impl Parsed {
    fn parse(args: &[String], value_opts: &[&str], flag_opts: &[&str]) -> Result<Parsed, String> {
        let mut p = Parsed { positional: vec![], values: HashMap::new(), flags: HashSet::new() };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a == "-" || !a.starts_with("--") {
                p.positional.push(a.clone());
            } else if let Some(name) = a.strip_prefix("--") {
                let (name, inline) = match name.split_once('=') {
                    Some((n, v)) => (n, Some(v.to_string())),
                    None => (name, None),
                };
                if flag_opts.contains(&name) {
                    p.flags.insert(name.to_string());
                } else if value_opts.contains(&name) {
                    let v = match inline {
                        Some(v) => v,
                        None => it.next().cloned().ok_or_else(|| format!("--{name} needs a value"))?,
                    };
                    p.values.insert(name.to_string(), v);
                } else {
                    return Err(format!("unknown option --{name}"));
                }
            }
        }
        Ok(p)
    }

    fn num<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, String> {
        match self.values.get(name) {
            None => Ok(None),
            Some(v) => v.parse().map(Some).map_err(|_| format!("--{name}: bad number {v:?}")),
        }
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    fn selection(&self) -> Result<Selection, String> {
        Ok(Selection {
            skip: self.num("skip")?.unwrap_or(0),
            limit: self.num("limit")?,
            only: match self.values.get("battle") {
                Some(s) => Some(Selection::parse_only(s)?),
                None => None,
            },
        })
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("replay") => cmd_replay(&args[1..]),
        Some("stats") => cmd_stats(&args[1..]),
        Some("selftest") => cmd_selftest(&args[1..]),
        Some("help") | Some("--help") | Some("-h") | None => {
            print!("{}", usage());
            return ExitCode::from(if args.is_empty() { 2 } else { 0 });
        }
        Some(other) => Err(format!("unknown command {other:?}\n\n{}", usage())),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("difftest: {e}");
            ExitCode::from(2)
        }
    }
}

fn cmd_replay(args: &[String]) -> Result<u8, String> {
    let p = Parsed::parse(
        args,
        &[
            "sim",
            "limit",
            "skip",
            "battle",
            "jobs",
            "show",
            "top",
            "context",
            "sig",
            "failed-out",
            "corrupt",
            "battle-timeout",
        ],
        &["stop-on-first", "json", "strict-order", "keep-indices", "full", "no-catch-panics", "quiet"],
    )?;
    let [input] = p.positional.as_slice() else {
        return Err(format!("replay needs exactly one input file (or -)\n\n{}", usage()));
    };
    let default_sim = if cfg!(feature = "engine") { "engine" } else { "mock" };
    let sim = p.values.get("sim").map_or(default_sim, String::as_str).to_string();
    let json = p.flag("json");
    let cfg = ReplayConfig {
        input: input.clone(),
        sim_name: sim.clone(),
        selection: p.selection()?,
        jobs: p.num("jobs")?.unwrap_or_else(default_jobs),
        stop_on_first: p.flag("stop-on-first"),
        opts: ReplayOpts {
            context: p.num("context")?.unwrap_or(15),
            strict_order: p.flag("strict-order"),
            catch_panics: !p.flag("no-catch-panics"),
            keep_indices: p.flag("keep-indices"),
            full: p.flag("full"),
        },
        progress: !p.flag("quiet") && !json,
        battle_timeout: match p.num::<u64>("battle-timeout")?.unwrap_or(120) {
            0 => None,
            s => Some(std::time::Duration::from_secs(s)),
        },
        repro_cmd: format!("difftest replay {input} --sim {sim}"),
    };
    if let Some(spec) = p.values.get("corrupt") {
        if sim != "mock" {
            return Err("--corrupt only works with --sim mock".into());
        }
        MockSim::set_global_corruption(Corruption::parse(spec)?);
    }
    let report: Report = match sim.as_str() {
        "mock" => run_replay::<MockSim>(&cfg),
        #[cfg(feature = "engine")]
        "engine" => run_replay::<difftest::engine_sim::EngineSim>(&cfg),
        other => {
            return Err(format!(
                "unknown or unavailable --sim {other:?} (mock{})",
                if cfg!(feature = "engine") { ", engine" } else { "; built without the `engine` feature" }
            ));
        }
    }
    .map_err(|e| format!("{input}: {e}"))?;

    if let Some(path) = p.values.get("failed-out") {
        let list: Vec<String> = report.failures.iter().map(|r| r.pos.to_string()).collect();
        std::fs::write(path, format!("{}\n", list.join(","))).map_err(|e| format!("{path}: {e}"))?;
    }
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if json {
        serde_json::to_writer(&mut out, &report).map_err(|e| e.to_string())?;
        let _ = writeln!(out);
    } else {
        let ro = RenderOpts {
            show: p.num("show")?.unwrap_or(5),
            sig_filter: p.values.get("sig").cloned(),
            repro_cmd: format!("difftest replay {input} --sim {sim}"),
            top: p.num("top")?.unwrap_or(40),
        };
        let _ = write!(out, "{}", render_human(&report, &ro));
    }
    Ok(u8::from(report.summary.failed > 0))
}

fn cmd_stats(args: &[String]) -> Result<u8, String> {
    let p = Parsed::parse(args, &["limit", "skip", "battle", "jobs", "top"], &["json"])?;
    let [input] = p.positional.as_slice() else {
        return Err(format!("stats needs exactly one input file (or -)\n\n{}", usage()));
    };
    let sel = p.selection()?;
    let jobs = p.num("jobs")?.unwrap_or_else(default_jobs);
    let stop = AtomicBool::new(false);
    let mut total = Stats::default();
    let t0 = Instant::now();
    for_each_line(input, &sel, jobs, &stop, |_, line| Stats::of_line(line), |_, s| total.merge(s))
        .map_err(|e| format!("{input}: {e}"))?;
    let wall = t0.elapsed().as_secs_f64();
    if p.flag("json") {
        let mut v = serde_json::to_value(&total).map_err(|e| e.to_string())?;
        v["wall_secs"] = serde_json::json!(wall);
        println!("{v}");
    } else {
        print!("{}", total.render(input, p.num("top")?.unwrap_or(30), wall));
    }
    Ok(0)
}

fn cmd_selftest(args: &[String]) -> Result<u8, String> {
    let p = Parsed::parse(args, &["limit"], &[])?;
    let mut fixtures: Vec<(String, Arc<Fixture>)> = vec![("synthetic".into(), Arc::new(selftest::synthetic_fixture()))];
    if let [path] = p.positional.as_slice() {
        let limit: usize = p.num("limit")?.unwrap_or(3);
        let stop = AtomicBool::new(false);
        let sel = Selection { limit: Some(limit), ..Selection::default() };
        let mut loaded = Vec::new();
        for_each_line(path, &sel, 1, &stop, |pos, line| (pos, Fixture::parse_line(line)), |_, r| loaded.push(r))
            .map_err(|e| format!("{path}: {e}"))?;
        loaded.sort_by_key(|(pos, _)| *pos);
        for (pos, fx) in loaded {
            fixtures.push((format!("battle {pos}"), Arc::new(fx.map_err(|e| format!("battle {pos}: {e}"))?)));
        }
    }
    let (mut ok, mut bad) = (0, 0);
    for (label, fx) in &fixtures {
        for (spec, res) in selftest::run_battery(fx) {
            match res {
                Ok(()) => {
                    ok += 1;
                    println!("ok    {label:>12}  {spec}");
                }
                Err(e) => {
                    bad += 1;
                    println!("FAIL  {label:>12}  {spec}: {e}");
                }
            }
        }
    }
    println!("\nselftest: {ok} passed, {bad} failed");
    Ok(u8::from(bad > 0))
}
