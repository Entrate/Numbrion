mod common;

use _numbrion::{
    action::*,
    env::{BatchEnv, Config, OnIllegal, SamplerParams, StepOut},
};
use common::*;
use std::sync::Arc;

fn make(n_envs: usize, threads: usize, seed: u64, log: bool, joint: bool, on_illegal: OnIllegal) -> BatchEnv {
    let cfg = Config { n_envs, seed, threads, log, joint_mask: joint, on_illegal };
    BatchEnv::new(cfg, Arc::new(test_pool())).expect("env")
}

/// Hundreds of complete battles through the Rust driver: no rejected choice, no panic, sane outcomes.
#[test]
fn random_self_play_never_rejects_or_panics() {
    let total = n_battles(400);
    let envs = 16;
    let mut env = make(envs, 4, 1, false, false, OnIllegal::Raise);
    let stats = env.run_random(total.div_ceil(envs) as u64, &SamplerParams::default()).expect("run");
    eprintln!("{stats:?}");
    assert!(stats.battles >= total as u64);
    assert_eq!(stats.illegal, 0);
    assert!(stats.turns / stats.battles >= 3, "battles end implausibly fast");
    assert!(stats.decisions > stats.battles * 5);
}

fn check_out(o: &StepOut) {
    const A: usize = N_ACTIONS;
    for i in 0..o.n {
        for s in 0..2 {
            let k = i * 2 + s;
            let needs = o.needs_action[k] == 1;
            let m0 = &o.mask0[k * A..(k + 1) * A];
            assert_eq!(needs, m0.iter().any(|&b| b == 1), "env {i} side {s}: needs_action vs mask0");
            assert_eq!(needs, o.request_kind[k] != 0);
            if !needs {
                assert!(o.mask1_any[k * A..(k + 1) * A].iter().all(|&b| b == 0));
            }
            if let Some(j) = &o.joint {
                let jm = &j[k * A * A..(k + 1) * A * A];
                for a0 in 0..A {
                    let row = &jm[a0 * A..(a0 + 1) * A];
                    assert_eq!(m0[a0] == 1, row.iter().any(|&b| b == 1), "env {i} side {s}: joint row {a0}");
                    for a1 in 0..A {
                        if row[a1] == 1 {
                            assert_eq!(o.mask1_any[k * A + a1], 1);
                        }
                    }
                }
            }
        }
        assert!((0..2).any(|s| o.needs_action[i * 2 + s] == 1), "env {i}: nobody to act");
        let (r0, r1) = (o.reward[i * 2], o.reward[i * 2 + 1]);
        if o.done[i] == 1 {
            assert!(o.final_turns[i] > 0);
            match o.winner[i] {
                0 => assert_eq!((r0, r1), (1.0, -1.0)),
                1 => assert_eq!((r0, r1), (-1.0, 1.0)),
                -1 => assert_eq!((r0, r1), (0.0, 0.0)),
                w => panic!("winner {w}"),
            }
        } else {
            assert_eq!((r0, r1, o.winner[i], o.final_turns[i]), (0.0, 0.0, -1, 0));
        }
    }
}

/// Drive `step` with the built-in sampler; collect a digest of everything it returns.
fn trajectory(threads: usize, steps: usize, seed: u64) -> (Vec<u8>, usize) {
    let mut env = make(8, threads, seed, false, true, OnIllegal::Raise);
    let mut digest = Vec::new();
    let mut dones = 0;
    let first = env.reset().unwrap();
    check_out(&first);
    for _ in 0..steps {
        let a = env.random_actions(&SamplerParams::default());
        let o = env.step(&a).unwrap();
        check_out(&o);
        dones += o.done.iter().map(|&d| d as usize).sum::<usize>();
        digest.extend_from_slice(&o.needs_action);
        digest.extend_from_slice(&o.mask0);
        digest.extend(o.reward.iter().flat_map(|r| r.to_le_bytes()));
        digest.extend(o.turn.iter().flat_map(|r| r.to_le_bytes()));
        digest.extend(o.battle_id.iter().flat_map(|r| r.to_le_bytes()));
    }
    (digest, dones)
}

#[test]
fn step_outputs_are_consistent_and_independent_of_thread_count() {
    let steps = 150;
    let (one, dones) = trajectory(1, steps, 5);
    let (four, _) = trajectory(4, steps, 5);
    let (other, _) = trajectory(4, steps, 6);
    assert!(dones > 0, "no battle finished in {steps} steps");
    assert!(one == four, "threads change the trajectory");
    assert!(one != other, "seed does not change the trajectory");
}

#[test]
fn illegal_actions_raise_or_fall_back_to_default() {
    // Raise: nothing is applied.
    let mut env = make(4, 2, 3, false, false, OnIllegal::Raise);
    let before = env.current();
    let bad = vec![N_ACTIONS as i32; 4 * 4];
    let err = env.step(&bad).unwrap_err();
    assert!(err.contains("not legal"), "{err}");
    let after = env.current();
    assert_eq!(before.turn, after.turn);
    assert_eq!(before.mask0, after.mask0);

    // Default: the engine's default choice is used and flagged; the game continues.
    let mut env = make(4, 2, 3, false, false, OnIllegal::Default);
    let mut flagged = 0;
    for _ in 0..30 {
        let o = env.step(&vec![-1; 4 * 4]).unwrap();
        check_out(&o);
        flagged += o.illegal.iter().map(|&x| x as usize).sum::<usize>();
    }
    assert!(flagged > 0);
}

#[test]
fn observations_are_player_views() {
    let mut env = make(2, 2, 9, true, false, OnIllegal::Raise);
    env.reset().unwrap();
    let mut seen = [[0usize; 2]; 2];
    let mut saw_terminal_log = false;
    for _ in 0..120 {
        let a = env.random_actions(&SamplerParams::default());
        let o = env.step(&a).unwrap();
        for e in 0..2 {
            for s in 0..2 {
                let obs = env.observe(e, s).unwrap();
                assert!(obs.request.is_some());
                seen[e][s] += obs.log.len();
                // Never an omniscient `split` marker, never an empty line.
                assert!(obs.log.iter().chain(&obs.prev_log).all(|l| !l.starts_with("|split|") && !l.is_empty()));
                if o.done[e] == 1 {
                    // The finished battle's last lines (win/tie) are delivered with the next observation.
                    assert!(obs.prev_log.iter().any(|l| l.starts_with("|win|") || l == "|tie"), "{:?}", obs.prev_log);
                    saw_terminal_log = true;
                }
            }
        }
    }
    assert!(seen.iter().flatten().all(|&n| n > 50));
    assert!(saw_terminal_log);
}

#[test]
fn mask_slot1_matches_joint_mask() {
    let mut env = make(6, 2, 4, false, true, OnIllegal::Raise);
    let mut o = env.reset().unwrap();
    for _ in 0..40 {
        // Pick the first legal slot-0 code of every side and compare against the joint matrix.
        let mut a0 = vec![-1i32; 6 * 2];
        for k in 0..12 {
            if let Some(a) = (0..N_ACTIONS).find(|&a| o.mask0[k * N_ACTIONS + a] == 1) {
                a0[k] = a as i32;
            }
        }
        let m1 = env.mask_slot1(&a0).unwrap();
        let j = o.joint.as_ref().unwrap();
        for k in 0..12 {
            for a1 in 0..N_ACTIONS {
                let expect = if a0[k] >= 0 { j[(k * N_ACTIONS + a0[k] as usize) * N_ACTIONS + a1] } else { 0 };
                assert_eq!(m1[k * N_ACTIONS + a1], expect);
            }
        }
        let a = env.random_actions(&SamplerParams::default());
        o = env.step(&a).unwrap();
    }
}
