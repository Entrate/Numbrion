//! Tests for the protect_redirection batch.
//!
//! * `scenarios.txt` holds whole recorded battles (raw log, turn and PRNG seed at every decision
//!   boundary) from tools/probes/protect_redirection/scenarios.mjs, driven through the pinned
//!   Showdown build. Each scenario replays against the Rust engine through the real core (events,
//!   lifecycle, choices, damage, mutators and text log), so the effect files of the batch are
//!   compared line by line and draw by draw (final and intermediate PRNG seeds) with the
//!   TypeScript they port.
//! * The `markers` test pins what each recorded scenario has to contain, so regenerating the
//!   table cannot silently drop the branch a scenario exists for.
//! * The `vectors` tests replay values read from the pinned handlers by
//!   tools/probes/protect_redirection/restart-vectors.mjs: Helping Hand restart stacking, Stall's
//!   counter tripling / cap and the StallMove draw sequence.
use super::*;
use crate::{
    Battle,
    actions::Attribution,
    log::{NoLog, TextLog},
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
    ("protect_stall_chain", &["|-activate|", "|-fail|"]),
    (
        "shields_contact_and_status",
        &["[from] Spiky Shield", "|-status|", "|-activate|"],
    ),
    ("wide_guard", &["move: Wide Guard"]),
    (
        "follow_me_rage_powder",
        &["move: Follow Me", "move: Rage Powder"],
    ),
    (
        "redirection_ignored_by_tracking",
        &["move: Follow Me", "move: Rage Powder"],
    ),
    (
        "lightning_rod_storm_drain",
        &[
            "ability: Lightning Rod",
            "ability: Storm Drain",
            "|-immune|",
        ],
    ),
    ("rage_powder_overcoat", &["move: Rage Powder"]),
    ("helping_hand_pollen_puff", &["Helping Hand", "|-heal|"]),
    (
        "pollen_puff_heal_block",
        &[
            "move: Heal Block",
            "|cant|p1a: Vivillon|move: Heal Block|Pollen Puff",
        ],
    ),
    ("ally_damage_modifiers", &["|-damage|"]),
    ("armor_tail", &["ability: Armor Tail"]),
    ("queenly_majesty", &["ability: Queenly Majesty"]),
    (
        "dragon_darts_into_protect",
        &["Dragon Darts", "|-singleturn|"],
    ),
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
    protect_stall_chain,
    shields_contact_and_status,
    wide_guard,
    follow_me_rage_powder,
    redirection_ignored_by_tracking,
    lightning_rod_storm_drain,
    rage_powder_overcoat,
    helping_hand_pollen_puff,
    ally_damage_modifiers,
    armor_tail,
    queenly_majesty,
    dragon_darts_into_protect
);

/// Psychic Noise puts Heal Block on the user of Pollen Puff before it moves: onTryMove stops the
/// ally heal (`cant|...|move: Heal Block|Pollen Puff`). The Heal Block condition itself belongs to
/// another effect batch, so this replay panics ("unimplemented effect hook") until that lands.
#[test]
#[ignore = "needs conditions:healblock (ported by another effect batch)"]
fn pollen_puff_heal_block() {
    replay("pollen_puff_heal_block");
}

// ---------------------------------------------------------------------------------
// Vectors from tools/probes/protect_redirection/restart-vectors.mjs (direct handler calls).
// ---------------------------------------------------------------------------------
fn vector_battle() -> Battle<TextLog> {
    // The exact teams of restart-vectors.mjs.
    let set = |species, ability, moves| {
        format!("|{species}||{ability}|{moves}|Serious|252,0,128,0,128,0|M|||100|,,,,,")
    };
    let p1 = [
        set("Clefairy", "Friend Guard", "followme,calmmind"),
        set("Garchomp", "Rough Skin", "earthquake,calmmind"),
    ]
    .join("]");
    let p2 = [
        set("Kingambit", "Defiant", "brickbreak,calmmind"),
        set("Dondozo", "Unaware", "calmmind,brickbreak"),
    ]
    .join("]");
    let mut b = Battle::from_players(
        [1, 2, 3, 4],
        ("Alice", &p1),
        ("Bob", &p2),
        TextLog::default(),
    )
    .unwrap();
    b.start().unwrap();
    let mut log = Vec::new();
    b.drain_log(&mut log);
    b
}
fn seed_text(b: &Battle<TextLog>) -> String {
    let s = b.seed();
    format!("{},{},{},{}", s[0], s[1], s[2], s[3])
}

#[test]
fn vectors_helping_hand_restart_stacks_the_multiplier() {
    let mut b = vector_battle();
    let (clefairy, garchomp) = (MonId(0), MonId(1));
    let seed_before = b.seed();
    for (expected, label) in [(1.5, "start"), (2.25, "restart"), (3.375, "second restart")] {
        let r = b.add_volatile(
            clefairy,
            dex::CONDITION_HELPINGHAND,
            Attribution::from_move(garchomp, EffectRef::None),
            None,
        );
        assert_eq!(r, Relay::Bool(true), "{label}");
        let cell = b
            .get_volatile(clefairy, dex::CONDITION_HELPINGHAND)
            .expect("helping hand volatile");
        let state = b.state.effects.cells[cell.0 as usize];
        let bits = u64::from(state.payload.words[0]) | (u64::from(state.payload.words[1]) << 32);
        assert_eq!(f64::from_bits(bits), expected, "{label}");
        assert_eq!(state.duration, 1, "{label}: duration stays 1");
    }
    let mut log = Vec::new();
    b.drain_log(&mut log);
    assert_eq!(
        log,
        vec!["|-singleturn|p1a: Clefairy|Helping Hand|[of] p1b: Garchomp"; 3]
    );
    assert_eq!(b.seed(), seed_before, "no PRNG draw");
}

#[test]
fn vectors_stall_counter_triples_up_to_729_and_resets_the_duration() {
    let mut b = vector_battle();
    let mon = MonId(7); // p2b: Dondozo
    for expected in [3u32, 9, 27, 81, 243, 729, 729, 729] {
        assert_eq!(
            b.add_volatile(mon, dex::CONDITION_STALL, Attribution::NONE, None),
            Relay::Bool(true)
        );
        let cell = b.get_volatile(mon, dex::CONDITION_STALL).unwrap();
        let state = b.state.effects.cells[cell.0 as usize];
        assert_eq!(state.payload.words[0], expected);
        assert_eq!(state.duration, 2);
    }
}

#[test]
fn vectors_stall_move_draws_one_random_chance_per_event() {
    let mut b = vector_battle();
    let mon = MonId(7);
    assert_eq!(
        seed_text(&b),
        "43514,9542,40559,8561",
        "start-of-battle seed"
    );
    b.remove_volatile(mon, dex::CONDITION_STALL);
    b.add_volatile(mon, dex::CONDITION_STALL, Attribution::NONE, None);
    // (counter before, success, seed after, volatile left) per StallMove event.
    let expected: [(u32, bool, &str, bool); 12] = [
        (3, true, "8769,17248,37115,18776", true),
        (3, false, "55250,62519,52978,42619", false),
        (3, false, "37673,46633,62039,8266", false),
        (3, false, "62891,40560,22227,62965", false),
        (3, false, "25495,32001,50713,50540", false),
        (3, false, "50886,5712,59628,20063", false),
        (3, false, "54274,43667,37867,24894", false),
        (3, true, "7104,42574,9152,10809", true),
        (3, true, "14735,5508,9502,51264", true),
        (9, true, "4124,63122,38965,57347", true),
        (9, false, "55173,41332,18014,39666", false),
        (3, true, "20898,18934,47363,16957", true),
    ];
    for (i, (counter, ok, seed_after, left)) in expected.into_iter().enumerate() {
        let cell = b.get_volatile(mon, dex::CONDITION_STALL).unwrap();
        assert_eq!(
            b.state.effects.cells[cell.0 as usize].payload.words[0],
            counter
        );
        let r = b.run_event(
            EventId::StallMove,
            EventArg::Holder(Holder::mon(mon)),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            RunEventOptions::default(),
        );
        assert_eq!(r, Relay::Bool(ok), "event {i}");
        assert_eq!(seed_text(&b), seed_after, "event {i}: PRNG seed");
        assert_eq!(
            b.get_volatile(mon, dex::CONDITION_STALL).is_some(),
            left,
            "event {i}: volatile"
        );
        if b.get_volatile(mon, dex::CONDITION_STALL).is_none() || i % 3 == 2 {
            b.add_volatile(mon, dex::CONDITION_STALL, Attribution::NONE, None);
        }
    }
}

// ---------------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------------
#[test]
fn type_named_matches_the_generated_type_table() {
    for (name, id) in [
        ("Electric", TYPE_ELECTRIC),
        ("Steel", TYPE_STEEL),
        ("Water", TYPE_WATER),
    ] {
        assert_eq!(crate::dex::type_id(name), Some(id), "{name}");
    }
}

#[test]
fn spa_plus_one_is_the_single_key_boost_object() {
    let b = spa_plus_one();
    assert_eq!((b.len, b.order[0], b.values[Stat::SpA as usize]), (1, 2, 1));
    assert_eq!(b.present, 1 << Stat::SpA as u8);
}

#[test]
fn the_all_target_exceptions_are_outside_the_scoped_dex() {
    // perishsong / flowershield / rototiller do not exist, so no scoped move is an exception.
    for key in ["perishsong", "flowershield", "rototiller"] {
        assert_eq!(dex::lookup(EffectKind::Move, key), None, "{key}");
    }
    assert!(!is_all_exception(dex::MOVE_PROTECT));
    assert!(!is_all_exception(EffectId::NONE));
}
