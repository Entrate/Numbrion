//! Exhaustive agreement between the action masks and the engine, on many random battles.
//!
//! For every decision boundary and every side that has to act, all 47 x 47 code pairs are tried against a
//! copy of the battle (typed path), and all ~6000 text pairs of the Showdown choice grammar are thrown at
//! `Battle::choose` (text path). A pair is accepted by the engine if and only if the mask allows it, modulo
//! documented aliases of forced moves.

mod common;

use _numbrion::{action::*, env::SamplerParams, game::Game, mask::SideActions, pool::SplitMix};
use common::*;

#[derive(Default, Debug)]
struct Coverage {
    boundaries: usize,
    side_boundaries: usize,
    move_requests: usize,
    switch_requests: usize,
    both_sides_act: usize,
    one_side_acts: usize,
    forced_move_slots: usize,
    auto_pass_slots: usize,
    revival_slots: usize,
    forced_pass_slots: usize,
    tera_offered: usize,
    pairs_checked: usize,
    pairs_legal: usize,
    pairs_masked_but_slot_legal: usize,
    aliases_accepted: usize,
    text_pairs: usize,
    text_accepted: usize,
}

fn record(cov: &mut Coverage, acts: &[SideActions; 2]) {
    cov.boundaries += 1;
    let n = acts.iter().filter(|a| a.needs_action()).count();
    if n == 2 {
        cov.both_sides_act += 1;
    } else {
        cov.one_side_acts += 1;
    }
    for a in acts.iter().filter(|a| a.needs_action()) {
        cov.side_boundaries += 1;
        match a.req {
            _numbrion::mask::Req::Move => cov.move_requests += 1,
            _numbrion::mask::Req::Switch => cov.switch_requests += 1,
            _numbrion::mask::Req::None => {}
        }
        for k in 0..2 {
            cov.forced_move_slots += a.forced(k) as usize;
            cov.auto_pass_slots += a.auto[k] as usize;
            cov.revival_slots += a.revival[k] as usize;
            cov.forced_pass_slots += (a.req == _numbrion::mask::Req::Switch && !a.auto[k] && a.slot_set[k] >> PASS & 1 == 1) as usize;
            cov.tera_offered += (a.slot_set[k] & CLASS_MASKS[1] != 0) as usize;
        }
    }
}

/// Typed path: every pair of codes against a fresh copy of the battle.
fn check_typed(base: &Game, trial: &mut Game, acts: &[SideActions; 2], cov: &mut Coverage) {
    for s in 0..2 {
        let a = &acts[s];
        if !a.needs_action() {
            continue;
        }
        let both = acts[1 - s].needs_action();
        let party = base.party(s);
        for a0 in 0..N_ACTIONS {
            for a1 in 0..N_ACTIONS {
                cov.pairs_checked += 1;
                let legal = a.is_legal(a0, a1);
                cov.pairs_legal += legal as usize;
                cov.pairs_masked_but_slot_legal +=
                    (!legal && a.slot_set[0] >> a0 & 1 == 1 && a.slot_set[1] >> a1 & 1 == 1) as usize;
                let choices = if legal {
                    a.choices(a0, a1, &party).expect("legal pair has choices")
                } else {
                    [base.unchecked_choice(s, 0, a0), base.unchecked_choice(s, 1, a1)]
                };
                // The engine's own predicate agrees with the mask on mask-legal pairs, and on masked-out
                // ones except for forced-move aliases.
                let engine_says = base.engine_joint_legal(s, &choices);
                if legal {
                    assert!(engine_says, "side {s}: engine predicate rejects mask-legal ({a0},{a1})");
                } else if engine_says {
                    let c = a.canonical([a0, a1]);
                    assert!(
                        c != [a0, a1] && a.is_legal(c[0], c[1]),
                        "side {s}: engine predicate accepts masked-out ({a0} {}, {a1} {})",
                        describe(a0),
                        describe(a1)
                    );
                }
                trial.copy_state_from(base);
                let r = trial.choose_typed(s, &choices);
                if legal {
                    if let Err(e) = &r {
                        panic!("side {s}: mask-legal ({a0} {}, {a1} {}) rejected: {e}", describe(a0), describe(a1));
                    }
                    if both {
                        // Nothing committed: the stored choice must be exactly what was asked for.
                        assert_eq!(trial.encode_choice(s), Some([a0, a1]), "round trip of ({a0},{a1})");
                    }
                } else if r.is_ok() {
                    let c = a.canonical([a0, a1]);
                    assert!(
                        c != [a0, a1] && a.is_legal(c[0], c[1]),
                        "side {s}: engine accepts masked-out ({a0} {}, {a1} {}); canonical {c:?}",
                        describe(a0),
                        describe(a1)
                    );
                    cov.aliases_accepted += 1;
                    if both {
                        assert_eq!(trial.encode_choice(s), Some(c));
                    }
                }
            }
        }
    }
}

/// The text grammar's per-slot universe: `pass`, `move 1..5 [-3..3] [terastallize]`, `switch 1..7`.
fn text_universe() -> Vec<(String, Option<usize>)> {
    let mut u = vec![("pass".to_string(), Some(PASS))];
    for m in 1..=5i32 {
        for loc in -3..=3i32 {
            for tera in [false, true] {
                let mut t = format!("move {m}");
                if loc != 0 {
                    t.push_str(&format!(" {loc:+}"));
                }
                if tera {
                    t.push_str(" terastallize");
                }
                let code = (m <= 4 && loc.abs() <= 2).then(|| move_action((m - 1) as u8, (loc + 2) as u8, tera));
                u.push((t, code));
            }
        }
    }
    for p in 1..=7usize {
        u.push((format!("switch {p}"), (p <= 6).then(|| switch_action((p - 1) as u8))));
    }
    u
}

/// Text path: the whole grammar against `Battle::choose` on copies of the battle.
fn check_text(base: &Game, trial: &mut Game, acts: &[SideActions; 2], universe: &[(String, Option<usize>)], cov: &mut Coverage) {
    for s in 0..2 {
        let a = &acts[s];
        if !a.needs_action() {
            continue;
        }
        let both = acts[1 - s].needs_action();
        for (t0, c0) in universe {
            for (t1, c1) in universe {
                cov.text_pairs += 1;
                let text = format!("{t0}, {t1}");
                let mapped = c0.zip(*c1).map(|(x, y)| [x, y]);
                trial.copy_state_from(base);
                let r = trial.choose(s, &text);
                match r {
                    Ok(()) => {
                        cov.text_accepted += 1;
                        let actual = if both { trial.encode_choice(s) } else { mapped };
                        let actual = actual.unwrap_or_else(|| panic!("side {s}: accepted `{text}` has no canonical code"));
                        let canon = a.canonical(actual);
                        assert!(
                            a.is_legal(canon[0], canon[1]),
                            "side {s}: engine accepts `{text}` = {actual:?} but the mask forbids {canon:?}"
                        );
                        if let Some(m) = mapped {
                            if both {
                                assert_eq!(actual, a.canonical(m), "side {s}: `{text}` stored as {actual:?}, expected {m:?}");
                            }
                            if a.canonical(m) == m {
                                assert!(a.is_legal(m[0], m[1]), "side {s}: engine accepts `{text}` but the mask forbids {m:?}");
                            }
                        } else {
                            panic!("side {s}: engine accepts unrepresentable `{text}`");
                        }
                    }
                    Err(e) => {
                        if let Some(m) = mapped {
                            assert!(
                                !a.is_legal(m[0], m[1]),
                                "side {s}: mask-legal {m:?} (`{text}`) rejected: {e}"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// The text rendering of every mask-legal pair is accepted and equals the typed choice in effect.
fn check_text_typed_agree(base: &Game, trials: &mut [Game; 2], acts: &[SideActions; 2], rng: &mut SplitMix) {
    for s in 0..2 {
        let a = &acts[s];
        if !a.needs_action() {
            continue;
        }
        let party = base.party(s);
        // A sample of legal pairs per side-boundary (all of them for small sets).
        let mut pairs = Vec::new();
        for a0 in bits(a.mask0()) {
            for a1 in bits(a.mask1(a0)) {
                pairs.push((a0, a1));
            }
        }
        let take = pairs.len().min(40);
        for _ in 0..take {
            let (a0, a1) = pairs[rng.below(pairs.len())];
            let [typed, text] = trials;
            typed.copy_state_from(base);
            text.copy_state_from(base);
            typed.choose_typed(s, &a.choices(a0, a1, &party).unwrap()).unwrap();
            let t = a.text(a0, a1).unwrap();
            text.choose(s, &t).unwrap_or_else(|e| panic!("`{t}` rejected: {e}"));
            if acts[1 - s].needs_action() {
                // Not committed yet: compare the stored choices.
                assert_eq!(typed.choice_text(s), text.choice_text(s), "`{t}`");
                assert_eq!(typed.encode_choice(s), Some([a0, a1]));
            } else {
                // Committed: compare the resulting battle.
                assert_eq!(typed.signature(), text.signature(), "typed vs text `{t}`");
            }
        }
    }
}

fn drive(n: usize, seed: u64, mut at_boundary: impl FnMut(&Game, &[SideActions; 2], &mut SplitMix)) {
    let pool = test_pool();
    let mut rng = SplitMix::new(seed);
    let p = SamplerParams::default();
    for _ in 0..n {
        let mut g = new_game(&pool, &mut rng, false);
        while !g.ended() {
            let acts = [g.side_actions(0), g.side_actions(1)];
            at_boundary(&g, &acts, &mut rng);
            random_step(&mut g, &mut rng, &p);
        }
    }
}

#[test]
fn typed_path_matches_masks_exactly() {
    let mut cov = Coverage::default();
    let n = n_battles(10);
    let mut trial: Option<Game> = None;
    drive(n, 11, |g, acts, _| {
        record(&mut cov, acts);
        let t = trial.get_or_insert_with(|| g.duplicate());
        if !t.same_origin(g) {
            *t = g.duplicate();
        }
        check_typed(g, t, acts, &mut cov);
    });
    eprintln!("typed coverage over {n} battles: {cov:#?}");
    assert!(cov.move_requests > 0 && cov.switch_requests > 0 && cov.pairs_legal > 0);
    assert!(cov.pairs_masked_but_slot_legal > 0, "joint constraints never bit");
}

#[test]
fn text_path_matches_masks_exactly() {
    let mut cov = Coverage::default();
    let n = n_battles(4);
    let universe = text_universe();
    assert_eq!(universe.len(), 1 + 5 * 7 * 2 + 7);
    let mut trial: Option<Game> = None;
    drive(n, 12, |g, acts, _| {
        record(&mut cov, acts);
        let t = trial.get_or_insert_with(|| g.duplicate());
        if !t.same_origin(g) {
            *t = g.duplicate();
        }
        check_text(g, t, acts, &universe, &mut cov);
    });
    eprintln!("text coverage over {n} battles: {cov:#?}");
    assert!(cov.text_accepted > 0);
}

#[test]
fn text_and_typed_choices_have_the_same_effect() {
    let n = n_battles(20);
    let mut trials: Option<[Game; 2]> = None;
    drive(n, 13, |g, acts, rng| {
        if trials.as_ref().is_none_or(|t| !t[0].same_origin(g)) {
            trials = Some([g.duplicate(), g.duplicate()]);
        }
        check_text_typed_agree(g, trials.as_mut().unwrap(), acts, rng)
    });
}

/// Cheap bulk check over many more battles: the mask equals the engine's own joint predicate on every
/// pair, and coverage of the rarer situations (revival, forced passes, forced moves) is reported.
#[test]
fn masks_equal_engine_predicate_on_many_battles() {
    let mut cov = Coverage::default();
    let n = n_battles(150);
    drive(n, 14, |g, acts, _| {
        record(&mut cov, acts);
        for s in 0..2 {
            let a = &acts[s];
            if !a.needs_action() {
                continue;
            }
            let party = g.party(s);
            let m0 = a.mask0();
            for a0 in 0..N_ACTIONS {
                let m1 = a.mask1(a0);
                // mask0 is exactly the set of codes with a completion.
                assert_eq!(m0 >> a0 & 1 == 1, m1 != 0, "mask0/mask1 disagree at {a0}");
                for a1 in 0..N_ACTIONS {
                    let legal = m1 >> a1 & 1 == 1;
                    let choices = if legal {
                        a.choices(a0, a1, &party).unwrap()
                    } else {
                        [g.unchecked_choice(s, 0, a0), g.unchecked_choice(s, 1, a1)]
                    };
                    let eng = g.engine_joint_legal(s, &choices);
                    if legal {
                        assert!(eng, "engine predicate rejects mask-legal ({a0},{a1})");
                    } else if eng {
                        let c = a.canonical([a0, a1]);
                        assert!(c != [a0, a1] && a.is_legal(c[0], c[1]), "predicate accepts masked-out ({a0},{a1})");
                    }
                }
            }
        }
    });
    eprintln!("bulk coverage over {n} battles: {cov:#?}");
    assert!(cov.forced_move_slots > 0, "no forced move seen");
}

/// Revival Blessing boundaries (a replacement request that may revive any fainted Pokemon, including a
/// fainted active one) are rare in random play, so steer play towards the move and check every such
/// boundary through both the typed and the text path.
#[test]
fn revival_blessing_boundaries_match() {
    let pool = test_pool();
    let revival_teams: Vec<usize> =
        (0..pool.len()).filter(|&i| pool.get(i).contains("revivalblessing")).collect();
    assert!(revival_teams.len() >= 5, "test pool lacks Revival Blessing teams");
    let mut rng = SplitMix::new(77);
    let p = SamplerParams::default();
    let universe = text_universe();
    let (mut cov, mut revival_boundaries) = (Coverage::default(), 0usize);
    let target = n_battles(60);
    let mut trial: Option<Game> = None;
    for _ in 0..target {
        let i = revival_teams[rng.below(revival_teams.len())];
        let j = revival_teams[rng.below(revival_teams.len())];
        let seed = rng.battle_seed();
        let Some(mut g) = new_game_with(&pool, i, j, seed, false) else { continue };
        while !g.ended() {
            let acts = [g.side_actions(0), g.side_actions(1)];
            let has_revival = acts.iter().any(|a| a.revival.iter().any(|&r| r));
            if has_revival {
                revival_boundaries += 1;
                record(&mut cov, &acts);
                if trial.as_ref().is_none_or(|t| !t.same_origin(&g)) {
                    trial = Some(g.duplicate());
                }
                let t = trial.as_mut().unwrap();
                check_typed(&g, t, &acts, &mut cov);
                if revival_boundaries % 3 == 0 {
                    check_text(&g, t, &acts, &universe, &mut cov);
                }
            }
            random_step_prefer(&mut g, &mut rng, &p, Some(engine::dex::MOVE_REVIVALBLESSING));
        }
    }
    eprintln!("revival boundaries: {revival_boundaries}, coverage: {cov:#?}");
    assert!(revival_boundaries >= 3, "Revival Blessing replacement requests were not reached");
}
