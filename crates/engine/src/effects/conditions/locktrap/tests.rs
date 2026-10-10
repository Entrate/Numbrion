//! Tests for the disable_lock_trap batch.
//!
//! * `scenarios.txt` holds complete battles replayed on the pinned Showdown
//!   (`tools/probes/disable_lock_trap/scenarios.mjs`): exact log lines and PRNG state after every decision
//!   boundary, plus explicit rejected choices (`bad || good`: every attempt before the last must be refused by
//!   the engine exactly as Showdown refused it, e.g. a switch by a trapped Pokemon). Together they reach every
//!   file of the batch: Choice lock (incl. the Dancer-copied move that BeforeMove refuses), Disable, Encore,
//!   Taunt, Imprison, Heal Block, Infestation (partiallytrapped), Spirit Shackle/trapped, No Retreat, Curse and
//!   the three trapping abilities.
//! * The unit tests below pin the payload conventions of the helper module.
use super::*;
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

const SCENARIOS: usize = 9;
/// Scenarios that also reach a hook owned by another effect batch which is not ported at this base
/// (`moves:pollenpuff` onTryMove/onTryHit/onHit, move_callbacks). They are replayed by the ignored test below
/// and must be enabled by the integrator once that batch lands.
const DEFERRED: &[&str] = &["healblock_pollenpuff"];

#[test]
fn scenario_file_is_well_formed() {
    let scenarios = parse_scenarios(include_str!("scenarios.txt"));
    assert_eq!(scenarios.len(), SCENARIOS);
    for sc in &scenarios {
        assert_eq!(sc.steps[0].label, "start");
        assert!(sc.steps.len() > 2, "{}", sc.name);
        assert!(sc.steps.iter().all(|s| !s.log.is_empty()), "{}", sc.name);
    }
    for name in DEFERRED {
        assert!(scenarios.iter().any(|s| s.name == *name), "{name}");
    }
}

/// Replays one scenario against the engine: exact log lines and PRNG state after every decision boundary, and
/// every `bad || good` attempt before the last one must be rejected.
fn replay(sc: &Scenario) {
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
            if choice.is_empty() {
                continue;
            }
            let attempts: Vec<&str> = choice.split(" || ").collect();
            for (i, attempt) in attempts.iter().enumerate() {
                let result = b.choose(side, attempt);
                if i + 1 == attempts.len() {
                    result.unwrap_or_else(|e| {
                        panic!(
                            "{} {}: side {side} {attempt:?} rejected: {e:?}",
                            sc.name, step.label
                        )
                    });
                } else {
                    assert!(
                        result.is_err(),
                        "{} {}: side {side} {attempt:?} must be rejected",
                        sc.name,
                        step.label
                    );
                }
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

/// The directed battles that only need hooks ported at this base. Set LOCKTRAP_SCENARIO=<name> to run one.
#[test]
fn scenarios_match_pinned_showdown() {
    let only = std::env::var("LOCKTRAP_SCENARIO").ok();
    for sc in parse_scenarios(include_str!("scenarios.txt")) {
        if only.as_deref().is_some_and(|n| n != sc.name) || DEFERRED.contains(&sc.name) {
            continue;
        }
        replay(&sc);
    }
}

#[test]
#[ignore = "needs moves:pollenpuff (move_callbacks batch)"]
fn deferred_scenarios_match_pinned_showdown() {
    for sc in parse_scenarios(include_str!("scenarios.txt")) {
        if DEFERRED.contains(&sc.name) {
            replay(&sc);
        }
    }
}

#[test]
fn type_ids_match_the_generated_type_table() {
    assert_eq!(dex::TYPE_NAMES[TYPE_GHOST.0 as usize - 1], "Ghost");
    assert_eq!(dex::TYPE_NAMES[TYPE_STEEL.0 as usize - 1], "Steel");
}

#[test]
fn stored_move_payload_round_trips_with_a_presence_bit() {
    use crate::{
        dex::HookId,
        event::{HookCtx, Relay},
        state::CellRef,
    };
    let mut b = Battle::new(
        [1, 2, 3, 4],
        "Pikachu|||static|thunderbolt|Serious||M|||100|,,,,,Electric",
        "Pikachu|||static|thunderbolt|Serious||M|||100|,,,,,Electric",
    )
    .unwrap();
    let cell = b.state.effects.alloc(
        Holder::mon(MonId(0)),
        Holder::mon(MonId(0)),
        dex::CONDITION_CHOICELOCK,
        0,
    );
    let state: CellRef = b.state.effects.capture(cell);
    let cx = HookCtx {
        frame: 255,
        call_args: 255,
        active_move: 255,
        state,
        holder: Holder::mon(MonId(0)),
        target: crate::event::EventArg::Undefined,
        source: crate::event::EventArg::Undefined,
        source_effect: crate::event::EffectRef::None,
        relay: Relay::Undefined,
        target_index: 255,
    };
    let _: Option<HookId> = None;
    assert_eq!(stored_move(&b, cx), EffectId::NONE);
    store_move(&mut b, cx, dex::MOVE_STRUGGLE);
    assert_eq!(stored_move(&b, cx), dex::MOVE_STRUGGLE);
    // The shared source/sourceEffect/target accessors read their present bits, not the payload.
    assert_eq!(state_source(&b, cx), None);
    assert_eq!(state_source_effect(&b, cx), crate::event::EffectRef::None);
    assert_eq!(state_target(&b, cx), Some(MonId(0)));
}
