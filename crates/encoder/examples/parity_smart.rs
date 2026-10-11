//! Runs deterministic SmartAgent decisions on Python parity arrays and request-derived masks.
mod common;
use encoder::{dex::Dex, smart::{ObsRows, SmartAgent, SmartConfig}, layout::N_ACTIONS};
use std::{path::Path, time::Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: parity_smart REF_DIR OUT_DIR");
    let input = common::load(Path::new(&args[1]));
    let ids = input["ids"].i64(); let floats = input["floats"].f32(); let field = input["field"].f32();
    let action = input["action_features"].f32(); let token = input["token_features"].f32();
    let mask0 = input["mask0"].bools(); let mask1 = input["mask1_given_a0"].bools();
    let boundaries = input["ids"].shape[0]; let n = input["ids"].shape[1];
    let rows: Vec<usize> = (0..n).collect();
    let dex = Dex::shared(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/training/dex.json"))).unwrap();
    let mut agent = SmartAgent::new(dex, 0, SmartConfig { temperature: 0.0, noise: 0.0, protect_skip: 0.0, protect_share: (0.7, 0.7) });
    let mut a0 = Vec::with_capacity(boundaries * n); let mut a1 = Vec::with_capacity(boundaries * n);
    let mut elapsed = std::time::Duration::ZERO;
    for t in 0..boundaries {
        let obs = ObsRows { ids: boundary(&ids, t, boundaries), floats: boundary(&floats, t, boundaries),
            field: boundary(&field, t, boundaries), action_features: boundary(&action, t, boundaries),
            token_features: boundary(&token, t, boundaries) };
        let start = Instant::now();
        let first = agent.first(&obs, &rows, &mask0[t * n * N_ACTIONS..(t + 1) * n * N_ACTIONS]);
        let mut second_mask = vec![false; n * N_ACTIONS];
        for r in 0..n {
            let offset = ((t * n + r) * N_ACTIONS + first[r] as usize) * N_ACTIONS;
            second_mask[r * N_ACTIONS..(r + 1) * N_ACTIONS].copy_from_slice(&mask1[offset..offset + N_ACTIONS]);
        }
        let second = agent.second(&second_mask);
        elapsed += start.elapsed(); a0.extend(first); a1.extend(second);
    }
    let mut writer = common::Writer::new(Path::new(&args[2]));
    writer.i32("a0", &[boundaries, n], &a0); writer.i32("a1", &[boundaries, n], &a1); writer.finish();
    println!("Rust first+second: {:.3} us/row ({} rows, {:.3}s)", elapsed.as_secs_f64() * 1e6 / (boundaries * n) as f64,
        boundaries * n, elapsed.as_secs_f64());
}

fn boundary<T>(values: &[T], t: usize, n: usize) -> &[T] {
    let stride = values.len() / n; &values[t * stride..(t + 1) * stride]
}
