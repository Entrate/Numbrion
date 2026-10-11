//! Rust-only request parsing / encode_row benchmark, with fixture I/O outside the timing.
//! Generate fixtures with pyengine's encoder_observations example (or parity/observations.py), then:
//! cargo run -p encoder --release --locked --example request_bench -- OBS.jsonl [REPEATS]
//! Reports the median of REPEATS (default 7) full-corpus passes; caches start empty each pass.
use encoder::{Dex, encode::*, json::Json};
use std::{hint::black_box, io::BufRead, path::Path, time::Instant};

type Boundary = Vec<(i64, String, Vec<String>)>;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().ok_or("usage: request_bench OBS.jsonl [REPEATS]")?;
    let repeats = args.get(1).map_or(Ok(7), |n| n.parse::<usize>()).map_err(|e| e.to_string())?;
    if repeats == 0 { return Err("REPEATS must be positive".into()); }
    let file = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
    let mut boundaries: Vec<Boundary> = Vec::new();
    for line in file.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() { continue; }
        let v = Json::parse(&line)?;
        boundaries.push(v.at("observations").items().iter().map(|o| {
            (o.at("battle_id").as_f64().unwrap_or(0.0) as i64, o.at("request").str_or_empty().to_string(),
             o.at("log").items().iter().map(|l| l.str_or_empty().to_string()).collect())
        }).collect());
    }
    let rows = boundaries.first().ok_or("empty corpus")?.len();
    if rows == 0 || boundaries.iter().any(|b| b.len() != rows) { return Err("inconsistent row counts".into()); }
    let dex = Dex::shared(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json"))?;
    let mut ids = vec![0; rows * IDS_PER_ROW];
    let mut floats = vec![0.0; rows * FLOATS_PER_ROW];
    let mut field = vec![0.0; rows * FIELD_PER_ROW];
    let mut foe_match = vec![0; rows * FOE_MATCH_PER_ROW];
    let mut fainted = vec![0.0; rows * FAINTED_PER_ROW];
    let mut guard = vec![false; rows * GUARD_PER_ROW];
    let mut action_features = vec![0.0; rows * ACTION_PER_ROW];
    let mut token_features = vec![0.0; rows * TOKEN_FEATURES_PER_ROW];
    let mut out = Outputs { ids: &mut ids, floats: &mut floats, field: &mut field, foe_match: &mut foe_match,
        fainted: &mut fainted, guard: &mut guard, action_features: &mut action_features, token_features: &mut token_features };
    let mut parses = Vec::new();
    let mut encodes = Vec::new();
    for _ in 0..repeats {
        let start = Instant::now();
        for b in &boundaries {
            for (_, request, _) in b {
                if !request.is_empty() { black_box(Json::parse(black_box(request))?); }
            }
        }
        parses.push(start.elapsed().as_secs_f64());
        let mut encoder = Encoder::new(dex.clone(), rows);
        let start = Instant::now();
        for b in &boundaries {
            for (row, (battle_id, request, log)) in b.iter().enumerate() {
                encoder.encode_row(row, &Observation { battle_id: *battle_id, request: Some(request), log }, &mut out)?;
            }
            encoder.match_partners(&mut out);
            black_box(&out);
        }
        encodes.push(start.elapsed().as_secs_f64());
    }
    parses.sort_by(f64::total_cmp);
    encodes.sort_by(f64::total_cmp);
    let count = boundaries.len() * rows;
    println!("{count} rows, {repeats} passes (median): parse {:.3} us/row; parse+encode {:.3} us/row (encode_row + partners, no damage)",
        parses[repeats / 2] * 1e6 / count as f64, encodes[repeats / 2] * 1e6 / count as f64);
    Ok(())
}
