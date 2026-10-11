//! Parity check of the damage features: reads `ids`, `floats` and `field` (`[T, R, ...]`) from a raw array directory
//! (`parity/arrays.py`), runs [`DamageFeatures::row`] on every row and writes `action_features` and
//! `token_features` with the same leading shape, for `parity/compare.py` against the Python reference.
//!
//! Usage: `cargo run -p encoder --example parity_damage --offline -- REF_DIR OUT_DIR [REPEATS]`; REPEATS (default 1)
//! reruns all rows that many times for the timing printed on stderr.

mod common;

use encoder::{damage::DamageFeatures, dex::Dex, layout::*};
use std::{path::Path, time::Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: parity_damage REF_DIR OUT_DIR [REPEATS]");
        std::process::exit(2);
    }
    let (src, out) = (Path::new(&args[1]), Path::new(&args[2]));
    let repeats: usize = args.get(3).map_or(1, |r| r.parse().expect("REPEATS"));
    let arrays = common::load(src);
    let ids = arrays["ids"].i64();
    let floats = arrays["floats"].f32();
    let field = arrays["field"].f32();
    let lead = &arrays["ids"].shape[..2];
    let rows = lead[0] * lead[1];
    assert_eq!(ids.len(), rows * N_TOKENS * N_IDS);
    assert_eq!(floats.len(), rows * N_TOKENS * N_POKEMON_FLOATS);
    assert_eq!(field.len(), rows * N_FIELD_FLOATS);

    let dex = Dex::shared(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json")).expect("dex.json");
    let damage = DamageFeatures::new(&dex);
    let (a_len, t_len) = (2 * N_CODES * N_ACTION_FEATURES, N_TOKENS * N_TOKEN_FEATURES);
    let mut action = vec![0.0f32; rows * a_len];
    let mut token = vec![0.0f32; rows * t_len];
    let started = Instant::now();
    for _ in 0..repeats {
        for r in 0..rows {
            damage.row(
                &ids[r * N_TOKENS * N_IDS..(r + 1) * N_TOKENS * N_IDS],
                &floats[r * N_TOKENS * N_POKEMON_FLOATS..(r + 1) * N_TOKENS * N_POKEMON_FLOATS],
                &field[r * N_FIELD_FLOATS..(r + 1) * N_FIELD_FLOATS],
                &mut action[r * a_len..(r + 1) * a_len],
                &mut token[r * t_len..(r + 1) * t_len],
            );
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    eprintln!("{rows} rows x {repeats} in {seconds:.3}s: {:.2} us/row", 1e6 * seconds / (rows * repeats) as f64);

    // The public pieces the smart player uses (tokens, foe_attacks against the 12 tokens, expected_max, any_ko) must
    // give the own tokens' threat columns of `row`.
    let mut mismatches = 0;
    for r in 0..rows {
        let (ids, floats) = (&ids[r * N_TOKENS * N_IDS..], &floats[r * N_TOKENS * N_POKEMON_FLOATS..]);
        let field = &field[r * N_FIELD_FLOATS..(r + 1) * N_FIELD_FLOATS];
        let t = damage.tokens(ids, floats, field);
        let foe = damage.foe_attacks(ids, floats, &t, &t, field);
        for i in 0..N_TOKENS {
            let present = floats[i * N_POKEMON_FLOATS + pf::PRESENT];
            if floats[i * N_POKEMON_FLOATS + pf::OWN] <= 0.5 {
                continue;
            }
            for f in 0..2 {
                let dealt: Vec<f32> = (0..encoder::damage::M_FOE).map(|m| foe.dealt(f, m, i)).collect();
                let ko: Vec<f32> = (0..encoder::damage::M_FOE).map(|m| foe.ko(f, m, i)).collect();
                let threat = encoder::damage::expected_max(&dealt, &foe.probs[f]) * present;
                let kos = encoder::damage::any_ko(&ko, &foe.probs[f]) * present;
                let out = &token[r * t_len + i * N_TOKEN_FEATURES..];
                if threat != out[f] || kos != out[2 + f] {
                    mismatches += 1;
                }
            }
        }
    }
    eprintln!("public API vs row threat columns: {mismatches} mismatches");

    let mut writer = common::Writer::new(out);
    writer.f32("action_features", &[lead[0], lead[1], 2, N_CODES, N_ACTION_FEATURES], &action);
    writer.f32("token_features", &[lead[0], lead[1], N_TOKENS, N_TOKEN_FEATURES], &token);
    writer.finish();
}
