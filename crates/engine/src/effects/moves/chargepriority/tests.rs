//! Tests of the `charge_priority_multihit` batch against the pinned Showdown.
//!
//! * `scenarios.txt` holds complete directed battles replayed on the oracle
//!   (`tools/probes/charge_priority_multihit/scenarios.mjs`): the exact log lines and PRNG state after every
//!   decision boundary. Together the scenarios reach every function site of the batch, including
//!   `lockedmove` (no scoped move creates it, so the probe applies the real
//!   `pokemon.addVolatile('lockedmove', null, move)` to both engines between turns, via `pre` records).
//! * Plain unit tests cover the helpers (type ids, boost object) and the payload encodings the choice code
//!   depends on.
//!
//! Regenerate with `node tools/probes/charge_priority_multihit/scenarios.mjs [path/to/pokemon-showdown]`.
use super::*;
use crate::{
    Battle,
    actions::Attribution,
    dex,
    event::EffectRef,
    ids::{EffectKind, MonId},
    log::TextLog,
};

struct Step {
    label: &'static str,
    pre: Vec<Vec<&'static str>>,
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
    let mut pending: Vec<Vec<&'static str>> = Vec::new();
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
            "pre" => pending.push(rest.split('\t').collect()),
            "step" => {
                let mut f = rest.split('\t');
                let label = f.next().unwrap();
                let c1 = f.next().unwrap_or("");
                let c2 = f.next().unwrap_or("");
                out.last_mut().unwrap().steps.push(Step {
                    label,
                    pre: std::mem::take(&mut pending),
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

const SCENARIOS: usize = 18;

#[test]
fn scenario_file_is_well_formed() {
    let scenarios = parse_scenarios(include_str!("scenarios.txt"));
    assert_eq!(scenarios.len(), SCENARIOS);
    for sc in &scenarios {
        assert_eq!(sc.steps[0].label, "start");
        assert!(sc.steps.len() > 2, "{}", sc.name);
        assert!(sc.steps.iter().all(|s| !s.log.is_empty()), "{}", sc.name);
    }
}

/// `pokemon.addVolatile(volatile, null, dex.moves.get(move))` from the probe, on the active Pokemon of
/// `side`/`slot` (no running event: the mutation boundary supplies the empty event).
fn apply_pre(b: &mut Battle<TextLog>, op: &[&str]) {
    assert_eq!(op[0], "addvolatile", "unknown scenario op {op:?}");
    let side: usize = if op[1] == "p1" { 0 } else { 1 };
    let slot: usize = op[2].parse().unwrap();
    let mon = b.state.sides[side].active[slot];
    assert_ne!(mon, MonId::NONE);
    let volatile = dex::lookup(EffectKind::Condition, op[3]).expect("condition");
    let mv = dex::lookup(EffectKind::Move, op[4]).expect("move");
    let result = b.add_volatile(
        mon,
        volatile,
        Attribution {
            source: crate::event::EventArg::Null,
            effect: EffectRef::Dex(mv),
        },
        None,
    );
    assert!(result.truthy(), "addVolatile {op:?} failed");
}

/// Replays every scenario in `text` (or just `only`) and compares the exact log lines and PRNG state after every
/// decision boundary.
fn replay(text: &'static str, only: Option<&str>) {
    for sc in parse_scenarios(text) {
        if only.is_some_and(|n| n != sc.name) {
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
            for op in &step.pre {
                apply_pre(&mut b, op);
            }
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

fn run_scenarios(only: Option<&str>) {
    replay(include_str!("scenarios.txt"), only);
}

/// Exact log lines and PRNG state after every decision boundary of the directed battles.
/// Set CPM_SCENARIO=<name> to run just one.
#[test]
fn scenarios_match_pinned_showdown() {
    run_scenarios(std::env::var("CPM_SCENARIO").ok().as_deref());
}

/// The same battles with the real Protect / Taunt / Helping Hand / Follow Me / Leech Seed / Rest / Roost / Spikes /
/// Tailwind / Knock Off / Super Fang in the slots that `scenarios.txt` fills with declarative stand-ins
/// (`SC_KEEP_OTHER_BATCH_MOVES=1 node tools/probes/charge_priority_multihit/scenarios.mjs --out .../scenarios_full.txt`).
/// Run it once the batches owning those callbacks (protect_redirection, disable_lock_trap, move_callbacks,
/// hazards_screens, healing_residual) are merged; it adds Phantom/Shadow Force breaking Protect, Fake Out into Protect,
/// Taunted Prankster users and so on.
#[test]
#[ignore = "needs the Protect/Taunt/Knock Off... callbacks of the other batches"]
fn scenarios_with_other_batch_moves_match_pinned_showdown() {
    replay(
        include_str!("scenarios_full.txt"),
        std::env::var("CPM_SCENARIO").ok().as_deref(),
    );
}

/// Every function site of the batch is invoked by the scenarios. The reached set is process-wide, so this must run
/// alone: `cargo test -p engine --lib --features trace-hooks -- --ignored scenarios_reach_every_hook --test-threads=1`.
#[cfg(feature = "trace-hooks")]
#[test]
#[ignore = "process-wide hook tracing: run alone with --test-threads=1"]
fn scenarios_reach_every_hook() {
    use crate::effects::{clear_reached_hooks, reached_hooks, registry as r};
    clear_reached_hooks();
    run_scenarios(None);
    let reached = reached_hooks();
    let mine: [&[crate::dex::HookId]; 24] = [
        r::moves_electroshot::HOOKS,
        r::moves_fakeout::HOOKS,
        r::moves_firstimpression::HOOKS,
        r::moves_highjumpkick::HOOKS,
        r::moves_meteorbeam::HOOKS,
        r::moves_phantomforce::HOOKS,
        r::moves_quash::HOOKS,
        r::moves_shadowforce::HOOKS,
        r::moves_solarbeam::HOOKS,
        r::moves_suckerpunch::HOOKS,
        r::moves_thunderclap::HOOKS,
        r::moves_tripleaxel::HOOKS,
        r::abilities_galewings::HOOKS,
        r::abilities_myceliummight::HOOKS,
        r::abilities_prankster::HOOKS,
        r::abilities_skilllink::HOOKS,
        r::abilities_triage::HOOKS,
        r::abilities_truant::HOOKS,
        r::items_loadeddice::HOOKS,
        r::items_powerherb::HOOKS,
        r::conditions_charge::HOOKS,
        r::conditions_lockedmove::HOOKS,
        r::conditions_mustrecharge::HOOKS,
        r::conditions_twoturnmove::HOOKS,
    ];
    let mut missing = Vec::new();
    let mut total = 0;
    for hooks in mine {
        for hook in hooks {
            total += 1;
            if !reached.contains(hook) {
                let h = &dex::HOOKS[hook.0 as usize];
                missing.push((dex::effect(h.effect).key, h.key));
            }
        }
    }
    assert_eq!(total, 41);
    assert!(missing.is_empty(), "hooks never reached: {missing:?}");
}

#[test]
fn type_constants_match_the_generated_type_table() {
    assert_eq!(TYPE_ELECTRIC, dex::type_id("Electric").unwrap());
    assert_eq!(TYPE_FLYING, dex::type_id("Flying").unwrap());
}

#[test]
fn spa_boost_object_is_the_sparse_js_literal() {
    let o = spa_plus_one();
    assert_eq!(o.len, 1);
    assert_eq!(o.order[0], 2);
    assert_eq!(o.values[2], 1);
    assert_eq!(o.present, 1 << 2);
}
