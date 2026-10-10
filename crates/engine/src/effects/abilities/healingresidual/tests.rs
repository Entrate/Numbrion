//! Tests for the healing_residual batch.
//!
//! * `scenarios.txt` holds whole recorded battles (raw log, turn and PRNG seed at every decision
//!   boundary) from tools/probes/healing_residual/scenarios.mjs, driven through the pinned
//!   Showdown build. Each scenario replays against the Rust engine through the real core (events,
//!   lifecycle, choices, damage, mutators and text log), so the effect files of the batch are
//!   compared line by line and draw by draw (final and intermediate PRNG seeds) with the
//!   TypeScript they port.
//! * The `marker` test pins what each recorded scenario has to contain, so regenerating the table
//!   cannot silently drop the branch a scenario exists for.
//! * The remaining tests cover the private helpers.
use super::*;
use crate::{
    Battle,
    log::{NoLog, TextLog},
    state::scratch::OrderedBoosts,
    teams::TeamDef,
};

const SCENARIOS: &str = include_str!("scenarios.txt");

enum Op<'a> {
    Boundary {
        turn: u32,
        seed: [u16; 4],
        log: Vec<&'a str>,
    },
    Choose {
        side: usize,
        text: &'a str,
    },
}
struct Scenario<'a> {
    name: &'a str,
    seed: [u16; 4],
    teams: [&'a str; 2],
    ops: Vec<Op<'a>>,
}
fn seed(text: &str) -> [u16; 4] {
    let v: Vec<u16> = text.split(',').map(|n| n.parse().unwrap()).collect();
    v.try_into().unwrap()
}
fn scenarios() -> Vec<Scenario<'static>> {
    let mut out: Vec<Scenario> = Vec::new();
    for line in SCENARIOS.lines().filter(|l| !l.starts_with('#')) {
        let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
        if tag == "S" {
            out.push(Scenario {
                name: rest,
                seed: [0; 4],
                teams: [""; 2],
                ops: Vec::new(),
            });
            continue;
        }
        let sc = out.last_mut().expect("scenario header first");
        match tag {
            "SEED" => sc.seed = seed(rest),
            "T1" => sc.teams[0] = rest,
            "T2" => sc.teams[1] = rest,
            "B" => {
                let (turn, s) = rest.split_once(' ').unwrap();
                sc.ops.push(Op::Boundary {
                    turn: turn.parse().unwrap(),
                    seed: seed(s),
                    log: Vec::new(),
                });
            }
            "L" => match sc.ops.last_mut() {
                Some(Op::Boundary { log, .. }) => log.push(rest),
                _ => panic!("log line outside a boundary"),
            },
            "C" => {
                let (side, text) = rest.split_once(' ').unwrap();
                sc.ops.push(Op::Choose {
                    side: usize::from(side == "p2"),
                    text,
                });
            }
            "E" => {}
            other => panic!("unknown scenario tag {other}"),
        }
    }
    out
}
fn normalize(line: &str) -> &str {
    let digits = line.strip_prefix("|t:|");
    if digits.is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())) {
        "|t:|"
    } else {
        line
    }
}
fn replay(name: &str) {
    let all = scenarios();
    let sc = all.iter().find(|s| s.name == name).expect("scenario");
    let mut b = Battle::from_players(
        sc.seed,
        ("Alice", sc.teams[0]),
        ("Bob", sc.teams[1]),
        TextLog::default(),
    )
    .unwrap();
    b.start().unwrap();
    for op in &sc.ops {
        match op {
            Op::Boundary { turn, seed, log } => {
                let mut got = Vec::new();
                b.drain_log(&mut got);
                let got: Vec<&str> = got.iter().map(|l| normalize(l)).collect();
                let first = got.iter().zip(log).position(|(a, e)| a != e);
                assert!(
                    first.is_none() && got.len() == log.len(),
                    "{name} before turn {turn}: first differing line {first:?} (got {} lines, want {}): got {:?}, want {:?}",
                    got.len(),
                    log.len(),
                    first.map(|i| got[i]),
                    first.map(|i| log[i]),
                );
                assert_eq!(b.seed(), *seed, "{name}: PRNG seed at turn {turn}");
                assert_eq!(b.turn(), *turn, "{name}: turn counter");
            }
            Op::Choose { side, text } => b
                .choose(*side, text)
                .unwrap_or_else(|e| panic!("{name}: side {side} choice {text:?}: {e:?}")),
        }
    }
}

#[test]
fn scenario_file_is_well_formed_and_every_team_is_in_scope() {
    let all = scenarios();
    assert_eq!(all.len(), MARKERS.len());
    for sc in &all {
        for team in sc.teams {
            TeamDef::unpack(team).unwrap_or_else(|e| panic!("{}: {e}", sc.name));
        }
        Battle::from_players(sc.seed, ("Alice", sc.teams[0]), ("Bob", sc.teams[1]), NoLog)
            .unwrap_or_else(|e| panic!("{}: {e}", sc.name));
        let boundaries = sc
            .ops
            .iter()
            .filter(|o| matches!(o, Op::Boundary { .. }))
            .count();
        assert!(boundaries >= 4, "{}: {boundaries} boundaries", sc.name);
        assert!(matches!(sc.ops.first(), Some(Op::Boundary { turn: 1, .. })));
    }
}

/// What each recorded scenario must contain (a substring of at least one of its log lines).
const MARKERS: &[(&str, &[&str])] = &[
    (
        "orbs_leftovers_poisonheal",
        &[
            "[from] item: Toxic Orb",
            "[from] item: Flame Orb",
            "[from] item: Leftovers",
            "[from] ability: Poison Heal",
        ],
    ),
    (
        "leech_seed_drain_and_grass_immune",
        &["move: Leech Seed", "|-immune|"],
    ),
    (
        "leech_seed_slot_after_switch",
        &["move: Leech Seed", "|-heal|"],
    ),
    (
        "wish_heals_replacement",
        &["[from] move: Wish|[wisher] Wishy"],
    ),
    ("wish_in_place_and_repeat", &["[from] move: Wish|[wisher]"]),
    (
        "natural_cure_ambiguous_switch",
        &["cured by Natural Cure.)"],
    ),
    (
        "natural_cure_and_regenerator",
        &["[from] ability: Natural Cure|[silent]"],
    ),
    ("healer_ally_status", &["ability: Healer", "|-curestatus|"]),
    (
        "shed_skin_statused",
        &["ability: Shed Skin", "|-curestatus|"],
    ),
    (
        "bad_dreams_sleeping_and_comatose",
        &["[from] ability: Bad Dreams"],
    ),
    (
        "dry_skin_water_rain_fire",
        &["[from] ability: Dry Skin", "|-immune|"],
    ),
    ("dry_skin_sun_damage", &["[from] ability: Dry Skin"]),
    ("salt_cure_water_steel", &["Salt Cure"]),
    ("syrup_bomb_speed_drops", &["Syrup Bomb", "|-end|"]),
    ("syrup_bomb_source_leaves", &["Syrup Bomb|[silent]"]),
    ("heal_pulse_and_jungle_healing", &["|-heal|", "|-fail|"]),
    (
        "lunar_blessing_take_heart_strength_sap",
        &["|-heal|", "|-boost|", "|-curestatus|"],
    ),
    ("hospitality_on_entry", &["[from] ability: Hospitality"]),
];

#[test]
fn every_scenario_contains_the_branch_it_exists_for() {
    let all = scenarios();
    for (name, markers) in MARKERS {
        let sc = all
            .iter()
            .find(|s| s.name == *name)
            .unwrap_or_else(|| panic!("scenario {name} missing"));
        for marker in *markers {
            let found = sc.ops.iter().any(|op| match op {
                Op::Boundary { log, .. } => log.iter().any(|line| line.contains(marker)),
                Op::Choose { .. } => false,
            });
            assert!(found, "{name}: no log line contains {marker:?}");
        }
    }
}
macro_rules! scenario_tests {
    ($($name:ident),*) => {$(
        #[test]
        fn $name() {
            replay(stringify!($name));
        }
    )*};
}
scenario_tests!(
    orbs_leftovers_poisonheal,
    leech_seed_drain_and_grass_immune,
    leech_seed_slot_after_switch,
    wish_heals_replacement,
    wish_in_place_and_repeat,
    natural_cure_ambiguous_switch,
    natural_cure_and_regenerator,
    healer_ally_status,
    shed_skin_statused,
    bad_dreams_sleeping_and_comatose,
    dry_skin_water_rain_fire,
    dry_skin_sun_damage,
    salt_cure_water_steel,
    syrup_bomb_speed_drops,
    syrup_bomb_source_leaves,
    heal_pulse_and_jungle_healing,
    lunar_blessing_take_heart_strength_sap,
    hospitality_on_entry
);

// ---------------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------------
#[test]
fn type_named_matches_the_generated_type_table() {
    for (name, id) in [
        ("Fire", TYPE_FIRE),
        ("Grass", TYPE_GRASS),
        ("Steel", TYPE_STEEL),
        ("Water", TYPE_WATER),
    ] {
        assert_eq!(crate::dex::type_id(name), Some(id), "{name}");
    }
}

#[test]
fn boosts_keep_object_key_order_not_stat_order() {
    // `{ spd: 1, spa: 1 }` iterates spd first (Object.keys order), whatever the stat index order.
    let b: OrderedBoosts = boosts([(Stat::SpD, 1), (Stat::SpA, 1)]);
    assert_eq!(b.len, 2);
    assert_eq!(b.order[..2], [Stat::SpD as u8, Stat::SpA as u8]);
    assert_eq!(b.values[Stat::SpD as usize], 1);
    assert_eq!(b.values[Stat::SpA as usize], 1);
    assert_eq!(b.present, (1 << Stat::SpD as u8) | (1 << Stat::SpA as u8));
    let one = boosts([(Stat::Atk, -1)]);
    assert_eq!((one.len, one.order[0], one.values[0]), (1, 0, -1));
}

#[test]
fn relay_amount_reads_a_damage_result_like_js_coercion() {
    assert_eq!(relay_amount(Relay::Number(37.0)), 37.0);
    // `heal(true)` (a zero-damage hit reported as `true`) coerces to 1.
    assert_eq!(relay_amount(Relay::Bool(true)), 1.0);
}
