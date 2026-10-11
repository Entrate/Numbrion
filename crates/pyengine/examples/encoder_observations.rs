//! Capture real requests and player logs for the encoder's Rust-only request_bench/parity_encode examples.
//! cargo run -p pyengine --release --locked --example encoder_observations -- OUT.jsonl [BOUNDARIES]
use _numbrion::{env::{BatchEnv, Config, OnIllegal, SamplerParams}, pool::TeamPool};
use std::{io::{BufWriter, Write}, sync::Arc};

fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c < ' ' => {
                use std::fmt::Write;
                write!(out, "\\u{:04x}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().ok_or("usage: encoder_observations OUT.jsonl [BOUNDARIES]")?;
    let boundaries = args.get(1).map_or(Ok(256), |n| n.parse::<usize>()).map_err(|e| e.to_string())?;
    let pool = Arc::new(TeamPool::load(&[concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/pool-400.txt.gz")], None)
        .map_err(|e| e.to_string())?);
    let mut env = BatchEnv::new(Config { n_envs: 16, seed: 731, threads: 1, log: true,
        joint_mask: false, on_illegal: OnIllegal::Raise }, pool)?;
    env.reset()?;
    let mut file = BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?);
    for _ in 0..boundaries {
        let mut rows = Vec::new();
        for e in 0..16 {
            for s in 0..2 {
                let o = env.observe(e, s)?;
                let log: Vec<String> = o.log.iter().map(|l| quoted(l)).collect();
                rows.push(format!("{{\"battle_id\":{},\"request\":{},\"log\":[{}]}}", o.battle_id,
                    o.request.as_deref().map_or_else(|| "null".into(), quoted), log.join(",")));
            }
        }
        writeln!(file, "{{\"observations\":[{}]}}", rows.join(",")).map_err(|e| e.to_string())?;
        let actions = env.random_actions(&SamplerParams::default());
        env.step(&actions)?;
    }
    file.flush().map_err(|e| e.to_string())?;
    println!("captured {} rows -> {path}", boundaries * 32);
    Ok(())
}
