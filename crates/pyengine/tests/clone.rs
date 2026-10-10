mod common;

use _numbrion::{env::SamplerParams, pool::SplitMix};
use common::*;

/// A duplicate taken at any decision boundary plays out exactly like the original when fed the same
/// decisions: same requests, same PRNG state, same player and omniscient log lines, same result.
#[test]
fn duplicates_play_out_identically() {
    let pool = test_pool();
    let mut rng = SplitMix::new(21);
    let p = SamplerParams::default();
    let (mut clones, mut compared_lines) = (0, 0);
    for b in 0..n_battles(25) {
        let mut g = new_game(&pool, &mut rng, true);
        let mut dup: Option<_> = None;
        while !g.ended() {
            // Take a copy at a random boundary (once per battle, plus the very first one for some).
            if dup.is_none() && (rng.below(8) == 0 || (b % 5 == 0 && g.turn() <= 1)) {
                let d = g.duplicate();
                assert_eq!(d.signature(), g.signature());
                // Align the log views: the copy starts with an empty log.
                g.drain_omni();
                g.drain_player(0);
                g.drain_player(1);
                dup = Some(d);
                clones += 1;
            }
            let codes = random_step_codes(&mut g, &mut rng, &p, None);
            if let Some(d) = dup.as_mut() {
                apply_codes(d, &codes);
                assert_eq!(d.signature(), g.signature(), "battle {b}: copy diverged");
                let (a, c) = (g.drain_omni(), d.drain_omni());
                assert_eq!(a, c, "battle {b}: omniscient log differs");
                compared_lines += a.len();
                for s in 0..2 {
                    assert_eq!(g.drain_player(s), d.drain_player(s), "battle {b}: player {s} log differs");
                }
            }
        }
        if let Some(d) = dup {
            assert_eq!(d.outcome().map(|o| (o.winner, o.turns)), g.outcome().map(|o| (o.winner, o.turns)));
        }
    }
    eprintln!("{clones} copies, {compared_lines} omniscient lines compared");
    assert!(clones > 3);
}

/// Reseeding changes what happens next; reseeding with the current state changes nothing.
#[test]
fn reseed_replaces_the_prng() {
    let pool = test_pool();
    let mut rng = SplitMix::new(31);
    let p = SamplerParams::default();
    let mut differing = 0;
    let mut trials = 0;
    for _ in 0..n_battles(12) {
        let mut g = new_game(&pool, &mut rng, false);
        for _ in 0..3 {
            if g.ended() {
                break;
            }
            random_step(&mut g, &mut rng, &p);
        }
        if g.ended() {
            continue;
        }
        let mut same = g.duplicate();
        let mut other = g.duplicate();
        same.reseed(g.prng_seed());
        other.reseed([1, 2, 3, 4]);
        assert_eq!(other.prng_seed(), [1, 2, 3, 4]);
        // Same decisions in all three.
        let mut r1 = SplitMix::new(1000);
        let mut done = false;
        while !g.ended() && !done {
            let codes = random_step_codes(&mut g, &mut r1, &p, None);
            apply_codes(&mut same, &codes);
            if !other.ended() {
                // `other` may legitimately reach different boundaries; follow it only while its choices stay legal.
                let ok = codes.iter().all(|&(s, a0, a1)| {
                    other.side_actions(s).is_legal(a0, a1)
                });
                if ok {
                    apply_codes(&mut other, &codes);
                } else {
                    done = true;
                }
            }
            assert_eq!(g.signature(), same.signature());
        }
        trials += 1;
        differing += (other.prng_seed() != g.prng_seed() || other.outcome().map(|o| o.turns) != g.outcome().map(|o| o.turns)) as usize;
    }
    assert!(trials >= 5 && differing >= trials / 2, "reseeding had no visible effect ({differing}/{trials})");
}
