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

/// Reuse a dirty worker at every boundary, including nonempty mid-turn queues.
#[test]
fn restored_workers_match_at_random_and_replacement_boundaries() {
    let pool = test_pool();
    let revival: Vec<_> = (0..pool.len()).filter(|&i| pool.get(i).contains("revivalblessing")).collect();
    let mut rng = SplitMix::new(77);
    let p = SamplerParams::default();
    let (mut boundaries, mut pivots, mut faints, mut revives) = (0, 0, 0, 0);
    for b in 0..n_battles(60) {
        let i = revival[rng.below(revival.len())];
        let j = revival[rng.below(revival.len())];
        let Some(mut g) = new_game_with(&pool, i, j, rng.battle_seed(), true) else { continue };
        let mut worker = g.duplicate();
        while !g.ended() {
            let acts = [g.side_actions(0), g.side_actions(1)];
            revives += acts.iter().any(|a| a.revival.iter().any(|&r| r)) as usize;
            if acts.iter().any(|a| a.req == _numbrion::mask::Req::Switch) {
                let active = g.state().sides.iter().flat_map(|s| s.active).filter(|m| *m != engine::ids::MonId::NONE);
                if active.clone().any(|m| g.state().pokemon[m.0 as usize].hp == 0) {
                    faints += 1;
                } else if !acts.iter().any(|a| a.revival.iter().any(|&r| r)) {
                    pivots += 1;
                }
            }
            // Leave unread branch logs and a different PRNG/state in the worker.
            worker.copy_state_from(&g);
            worker.reseed([9, 8, 7, 6]);
            random_step(&mut worker, &mut rng, &p);
            worker.copy_state_from(&g);
            assert!(worker.drain_omni().is_empty());
            assert!(worker.drain_player(0).is_empty());
            assert!(worker.drain_player(1).is_empty());
            assert_eq!(worker.signature(), g.signature());
            g.drain_omni();
            g.drain_player(0);
            g.drain_player(1);
            let codes = random_step_codes(&mut g, &mut rng, &p, Some(engine::dex::MOVE_REVIVALBLESSING));
            apply_codes(&mut worker, &codes);
            assert_eq!(worker.signature(), g.signature(), "battle {b} boundary {boundaries}");
            assert_eq!(worker.drain_omni(), g.drain_omni());
            for side in 0..2 {
                assert_eq!(worker.drain_player(side), g.drain_player(side));
            }
            boundaries += 1;
        }
    }
    eprintln!("restore: {boundaries} boundaries, {pivots} pivots, {faints} faints, {revives} revivals");
    assert!(boundaries > 100 && pivots > 0 && faints > 0 && revives > 0);
}

#[test]
fn reset_worker_matches_fresh_games_with_new_teams_and_seeds() {
    let pool = test_pool();
    let mut rng = SplitMix::new(143);
    let p = SamplerParams::default();
    for log in [false, true] {
        let mut worker = new_game(&pool, &mut rng, log);
        for _ in 0..12 {
            let (i, j) = (rng.below(pool.len()), rng.below(pool.len()));
            let seed = rng.battle_seed();
            let Some(mut fresh) = new_game_with(&pool, i, j, seed, log) else { continue };
            worker.reset_from_team_defs(seed, [pool.get(i).clone(), pool.get(j).clone()],
                [pool.parsed(i).unwrap(), pool.parsed(j).unwrap()]).unwrap();
            worker.start().unwrap();
            assert_eq!(worker.signature(), fresh.signature());
            assert_eq!(worker.drain_omni(), fresh.drain_omni());
            for side in 0..2 { assert_eq!(worker.drain_player(side), fresh.drain_player(side)); }
            while !fresh.ended() {
                let codes = random_step_codes(&mut fresh, &mut rng, &p, None);
                apply_codes(&mut worker, &codes);
                assert_eq!(worker.signature(), fresh.signature());
                assert_eq!(worker.drain_omni(), fresh.drain_omni());
                for side in 0..2 { assert_eq!(worker.drain_player(side), fresh.drain_player(side)); }
            }
        }
    }
}
