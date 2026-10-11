//! Encodes every boundary of an observation JSONL (`parity/observations.py`) with one `Encoder` of all its rows and
//! writes `ids`, `floats`, `field`, `foe_match`, `fainted` and `guard` stacked `[boundaries, rows, ...]` (plus
//! `action_features` and `token_features` with `--damage`) for `parity/compare.py`.
//!
//! Usage: cargo run -p encoder --example parity_encode --offline -- OBS.jsonl OUT_DIR [--damage] [--repeat N]
//! `--repeat N` encodes the whole file N more times (fresh encoder each time) and reports the best time spent in the
//! encoder calls (JSON reading excluded), for timing.

mod common;

use encoder::{
    Dex,
    encode::*,
    json::Json,
    layout::{N_FIELD_FLOATS, N_IDS, N_POKEMON_FLOATS, N_TOKENS},
};
use std::{io::BufRead, path::Path, time::Instant};

struct Arrays {
    ids: Vec<i64>,
    floats: Vec<f32>,
    field: Vec<f32>,
    foe_match: Vec<i64>,
    fainted: Vec<f32>,
    guard: Vec<bool>,
    action_features: Vec<f32>,
    token_features: Vec<f32>,
}

impl Arrays {
    fn new(rows: usize) -> Self {
        Arrays {
            ids: vec![0; rows * IDS_PER_ROW],
            floats: vec![0.0; rows * FLOATS_PER_ROW],
            field: vec![0.0; rows * FIELD_PER_ROW],
            foe_match: vec![-1; rows * FOE_MATCH_PER_ROW],
            fainted: vec![0.0; rows * FAINTED_PER_ROW],
            guard: vec![false; rows * GUARD_PER_ROW],
            action_features: vec![0.0; rows * ACTION_PER_ROW],
            token_features: vec![0.0; rows * TOKEN_FEATURES_PER_ROW],
        }
    }

    fn outputs(&mut self) -> Outputs<'_> {
        Outputs {
            ids: &mut self.ids,
            floats: &mut self.floats,
            field: &mut self.field,
            foe_match: &mut self.foe_match,
            fainted: &mut self.fainted,
            guard: &mut self.guard,
            action_features: &mut self.action_features,
            token_features: &mut self.token_features,
        }
    }
}

/// One boundary: (battle_id, request text, log lines) per row.
type Boundary = Vec<(i64, String, Vec<String>)>;

fn parse(line: &str) -> Boundary {
    let v = Json::parse(line).expect("JSON line");
    v.at("observations")
        .items()
        .iter()
        .map(|o| {
            let battle_id = o.at("battle_id").as_f64().expect("battle_id") as i64;
            let request = o.at("request").str_or_empty().to_string();
            let log = o.at("log").items().iter().map(|l| l.str_or_empty().to_string()).collect();
            (battle_id, request, log)
        })
        .collect()
}

/// Encodes every boundary of the file (read one line at a time) with a fresh encoder; calls `keep` with the arrays
/// after each boundary. Returns (boundaries, rows, seconds spent in the encoder calls).
fn run(dex: &std::sync::Arc<Dex>, path: &Path, damage: bool, mut keep: impl FnMut(&Arrays)) -> (usize, usize, f64) {
    let file = std::io::BufReader::new(std::fs::File::open(path).expect("observation JSONL"));
    let mut state: Option<(Encoder, Arrays)> = None;
    let (mut n, mut rows, mut seconds) = (0, 0, 0.0);
    for line in file.lines() {
        let line = line.expect("read line");
        if line.trim().is_empty() {
            continue;
        }
        let boundary = parse(&line);
        rows = boundary.len();
        let (encoder, arrays) = state.get_or_insert_with(|| (Encoder::new(dex.clone(), rows), Arrays::new(rows)));
        let started = Instant::now();
        let mut out = arrays.outputs();
        for (row, (battle_id, request, log)) in boundary.iter().enumerate() {
            let obs = Observation { battle_id: *battle_id, request: Some(request.as_str()), log: log.as_slice() };
            encoder.encode_row(row, &obs, &mut out).expect("encode_row");
        }
        encoder.match_partners(&mut out);
        if damage {
            encoder.damage_features(&mut out);
        }
        seconds += started.elapsed().as_secs_f64();
        keep(arrays);
        n += 1;
    }
    (n, rows, seconds)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let damage = args.iter().any(|a| a == "--damage");
    let repeat = args.iter().position(|a| a == "--repeat").map_or(0, |i| args[i + 1].parse::<usize>().expect("--repeat N"));
    let positional: Vec<&String> =
        args.iter().enumerate().filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--repeat")).map(|(_, a)| a).collect();
    let [obs_path, out_dir] = positional[..] else {
        eprintln!("usage: parity_encode OBS.jsonl OUT_DIR [--damage] [--repeat N]");
        std::process::exit(2);
    };
    let dex = Dex::shared(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json")).expect("dex.json");
    let mut stacked = Arrays::new(0);
    let (n, rows, mut best) = run(&dex, Path::new(obs_path), damage, |a| {
        stacked.ids.extend_from_slice(&a.ids);
        stacked.floats.extend_from_slice(&a.floats);
        stacked.field.extend_from_slice(&a.field);
        stacked.foe_match.extend_from_slice(&a.foe_match);
        stacked.fainted.extend_from_slice(&a.fainted);
        stacked.guard.extend_from_slice(&a.guard);
        if damage {
            stacked.action_features.extend_from_slice(&a.action_features);
            stacked.token_features.extend_from_slice(&a.token_features);
        }
    });
    for _ in 0..repeat {
        best = best.min(run(&dex, Path::new(obs_path), damage, |_| {}).2);
    }

    let mut w = common::Writer::new(Path::new(out_dir));
    w.i64("ids", &[n, rows, N_TOKENS, N_IDS], &stacked.ids);
    w.f32("floats", &[n, rows, N_TOKENS, N_POKEMON_FLOATS], &stacked.floats);
    w.f32("field", &[n, rows, N_FIELD_FLOATS], &stacked.field);
    w.i64("foe_match", &[n, rows, 6], &stacked.foe_match);
    w.f32("fainted", &[n, rows, 2], &stacked.fainted);
    w.bools("guard", &[n, rows, 2, 4, 2], &stacked.guard);
    if damage {
        w.f32("action_features", &[n, rows, 2, encoder::layout::N_CODES, encoder::layout::N_ACTION_FEATURES], &stacked.action_features);
        w.f32("token_features", &[n, rows, N_TOKENS, encoder::layout::N_TOKEN_FEATURES], &stacked.token_features);
    }
    w.finish();
    println!(
        "{n} boundaries x {rows} rows in {best:.3}s ({:.4} ms/row{}) -> {out_dir}",
        1e3 * best / (n * rows) as f64,
        if damage { ", with damage features" } else { "" }
    );
}
