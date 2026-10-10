//! rules_status scenario replays against the pinned Showdown.
//!
//! `scenarios.txt` is produced by `tools/probes/rules_status/scenarios.mjs` (it plays directed
//! `gen9randomdoublesbattle` battles on the real simulator): per decision boundary it records the
//! choices, the normalised log lines and the PRNG state afterwards. The scenarios reach Yawn (move
//! and volatile: expiry, Sleep Clause, Electric Terrain / Insomnia / Vital Spirit / Comatose /
//! refusals, switching out), Rest (full HP, Insomnia, Vital Spirit, Comatose,
//! burn replaced, Early Bird, the fixed two sleeping turns) and the Dire Claw / Tri Attack status
//! lotteries (immune types, Sleep Clause, Shield Dust), on top of the six statuses, flinch and the
//! format rules which every battle exercises.
//!
//! Set `RULES_STATUS_SCENARIO=<name>` to run a single scenario.
use crate::{Battle, log::TextLog};

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

const SCENARIOS: &[&str] = &[
    "yawn_sleep_clause",
    "yawn_blockers",
    "rest_branches",
    "status_lotteries",
];

#[test]
fn scenario_file_is_well_formed() {
    let scenarios = parse_scenarios(include_str!("scenarios.txt"));
    let names: Vec<&str> = scenarios.iter().map(|s| s.name).collect();
    assert_eq!(names, SCENARIOS);
    for sc in &scenarios {
        assert_eq!(sc.steps[0].label, "start");
        assert!(sc.steps.len() > 8, "{}", sc.name);
        assert!(sc.steps.iter().all(|s| !s.log.is_empty()), "{}", sc.name);
    }
}

/// Replays the named scenarios, comparing the log lines and the PRNG state at every boundary.
fn replay(names: &[&str]) {
    let only = std::env::var("RULES_STATUS_SCENARIO").ok();
    for sc in parse_scenarios(include_str!("scenarios.txt")) {
        if !names.contains(&sc.name) || only.as_deref().is_some_and(|n| n != sc.name) {
            continue;
        }
        let mut b = Battle::from_players(
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
}

/// Yawn move + volatile: expiry, Sleep Clause, Electric Terrain / Insomnia / Vital Spirit / Comatose
/// refusals, switching out.
#[test]
fn yawn_scenarios_match_pinned_showdown() {
    replay(&["yawn_sleep_clause", "yawn_blockers"]);
}

/// Rest and the Dire Claw / Tri Attack lotteries. These scenarios switch statused Pokemon out on
/// purpose (to bring fresh targets in), and every voluntary switch of a statused Pokemon runs
/// `singleEvent('CheckShow', naturalcure)` (sim/battle.ts:2767), whose handler belongs to the
/// healing_residual batch. Verified passing bit-for-bit with that port applied.
#[test]
#[ignore = "needs abilities:naturalcure onCheckShow (healing_residual batch)"]
fn rest_and_lottery_scenarios_match_pinned_showdown() {
    replay(&["rest_branches", "status_lotteries"]);
}
