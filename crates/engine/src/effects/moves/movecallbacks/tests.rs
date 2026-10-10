//! Tests for the move_callbacks batch (compiled once, from `acrobatics.rs`).
//!
//! * `scenarios.txt` holds complete battles replayed on the pinned Showdown
//!   (`tools/probes/move_callbacks/scenarios.mjs`): exact log lines and PRNG state after every
//!   decision boundary. They target the branches the random directed corpus reaches rarely (see the
//!   header of the generator): Stomping Tantrum after a failed move, Roost's Terastallized and
//!   Transform paths, Hoopa forms for Hyperspace Fury, Substitute for Shell Side Arm, Belly Drum and
//!   Clangorous Soul thresholds under Contrary, Glaive Rush's drawback, Haze/Clear Smog, ...
//! * The pure table helpers are checked against hand-derived values from the TypeScript.
use super::support;
use crate::log::TextLog;

struct Step {
    label: &'static str,
    choices: [&'static str; 2],
    seed_after: [u16; 4],
    log: Vec<&'static str>,
}
struct Scenario {
    name: &'static str,
    seed: [u16; 4],
    teams: [&'static str; 2],
    steps: Vec<Step>,
}
fn parse_seed(text: &str) -> [u16; 4] {
    let v: Vec<u16> = text.split(',').map(|n| n.parse().unwrap()).collect();
    v.try_into().unwrap()
}
fn parse_scenarios(text: &'static str) -> Vec<Scenario> {
    let mut out: Vec<Scenario> = Vec::new();
    for line in text.lines() {
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "scenario" => out.push(Scenario {
                name: rest,
                seed: [0; 4],
                teams: [""; 2],
                steps: Vec::new(),
            }),
            "seed" => out.last_mut().unwrap().seed = parse_seed(rest),
            "p1" => out.last_mut().unwrap().teams[0] = rest,
            "p2" => out.last_mut().unwrap().teams[1] = rest,
            "step" => {
                let mut f = rest.split('\t');
                let label = f.next().unwrap();
                let c1 = f.next().unwrap_or("");
                let c2 = f.next().unwrap_or("");
                out.last_mut().unwrap().steps.push(Step {
                    label,
                    choices: [c1, c2],
                    seed_after: [0; 4],
                    log: Vec::new(),
                });
            }
            "seed_after" => {
                out.last_mut().unwrap().steps.last_mut().unwrap().seed_after = parse_seed(rest)
            }
            "log" => out
                .last_mut()
                .unwrap()
                .steps
                .last_mut()
                .unwrap()
                .log
                .push(rest),
            _ => panic!("unknown scenario record {line:?}"),
        }
    }
    out
}

/// Replays one scenario: the log lines and the PRNG state after every decision boundary must equal
/// the pinned Showdown's.
fn replay(name: &str) {
    let scenarios = parse_scenarios(include_str!("scenarios.txt"));
    let sc = scenarios
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("scenario {name} missing from scenarios.txt"));
    let mut b = crate::Battle::from_players(
        sc.seed,
        ("Alice", sc.teams[0]),
        ("Bob", sc.teams[1]),
        TextLog::default(),
    )
    .unwrap();
    b.start().unwrap();
    for step in &sc.steps {
        for (side, choice) in step.choices.iter().enumerate() {
            if !choice.is_empty() {
                b.choose(side, choice).unwrap_or_else(|e| {
                    panic!(
                        "{} {}: side {side} {choice:?} rejected: {e:?}",
                        sc.name, step.label
                    )
                });
            }
        }
        let mut got = Vec::new();
        b.drain_log(&mut got);
        let got: Vec<String> = got
            .into_iter()
            .map(|l| {
                if l.starts_with("|t:|") && l[4..].bytes().all(|c| c.is_ascii_digit()) {
                    "|t:|".to_owned()
                } else {
                    l
                }
            })
            .collect();
        assert_eq!(got, step.log, "{} {} log", sc.name, step.label);
        assert_eq!(b.seed(), step.seed_after, "{} {} seed", sc.name, step.label);
    }
}

macro_rules! scenario_tests {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                replay(stringify!($name));
            }
        )*
        const SCENARIO_NAMES: &[&str] = &[$(stringify!($name)),*];
    };
}
scenario_tests!(
    acrobatics_facade,
    stomping_tantrum_after_failure,
    avalanche_ragefist_lashout,
    last_respects_after_faints,
    hp_scaled_power_after_bellydrum,
    dragonenergy_after_clangoroussoul,
    bellydrum_gates,
    hyperspacefury_forms,
    doubleshock_types,
    doubleshock_tera,
    roost_types,
    roost_tera_flying,
    roost_tera_other,
    roost_transform,
    shellsidearm_targets,
    alluring_burning_after_boost,
    weight_moves,
    photon_geyser_boosts,
    fusion_pair_order,
    freezedry_water_targets,
    endeavor_gambit_fang_ruination,
    glaive_rush_drawback,
    haze_clearsmog,
);

/// Scenarios that cross another batch's effect (Substitute); they run unchanged once it is merged
/// (`cargo test -p engine -- --ignored substitute`).
macro_rules! pending_scenario_tests {
    ($(($name:ident, $reason:literal)),* $(,)?) => {
        $(
            #[test]
            #[ignore = $reason]
            fn $name() {
                replay(stringify!($name));
            }
        )*
        const PENDING_SCENARIO_NAMES: &[&str] = &[$(stringify!($name)),*];
    };
}
pending_scenario_tests!(
    (
        shellsidearm_substitute,
        "needs moves:substitute (another effect batch)"
    ),
    (
        clearsmog_substitute,
        "needs moves:substitute (another effect batch)"
    ),
);

#[test]
fn scenario_file_lists_exactly_the_tested_scenarios() {
    let scenarios = parse_scenarios(include_str!("scenarios.txt"));
    let names: Vec<&str> = scenarios.iter().map(|s| s.name).collect();
    let expected: Vec<&str> = SCENARIO_NAMES
        .iter()
        .chain(PENDING_SCENARIO_NAMES)
        .copied()
        .collect();
    assert_eq!(names, expected);
    for sc in &scenarios {
        assert_eq!(sc.steps[0].label, "start");
        assert!(sc.steps.len() > 2, "{}", sc.name);
        assert!(sc.steps.iter().all(|s| !s.log.is_empty()), "{}", sc.name);
    }
}

#[test]
fn target_weight_table_matches_grass_knot_and_low_kick() {
    // data/moves.ts:7544-7556 and 10449-10461: thresholds are inclusive lower bounds in hectograms.
    for (weight, bp) in [
        (1.0, 20.0),
        (99.0, 20.0),
        (100.0, 40.0),
        (249.0, 40.0),
        (250.0, 60.0),
        (499.0, 60.0),
        (500.0, 80.0),
        (999.0, 80.0),
        (1000.0, 100.0),
        (1999.0, 100.0),
        (2000.0, 120.0),
        (9999.0, 120.0),
    ] {
        assert_eq!(support::target_weight_base_power(weight), bp, "{weight}");
    }
}

#[test]
fn weight_ratio_table_matches_heavy_slam_and_heat_crash() {
    // data/moves.ts:8484-8494 and 8541-8551: `pokemonWeight >= targetWeight * k`.
    for (user, target, bp) in [
        (500.0, 100.0, 120.0),
        (499.0, 100.0, 100.0),
        (400.0, 100.0, 100.0),
        (399.0, 100.0, 80.0),
        (300.0, 100.0, 80.0),
        (299.0, 100.0, 60.0),
        (200.0, 100.0, 60.0),
        (199.0, 100.0, 40.0),
        (1.0, 1000.0, 40.0),
        (f64::NAN, 100.0, 40.0),
    ] {
        assert_eq!(
            support::weight_ratio_base_power(user, target),
            bp,
            "{user} vs {target}"
        );
    }
}
