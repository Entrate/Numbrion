use crate::{
    Battle,
    actions::{MoveHandle, Stat, StatOptions},
    dex::{self, Category},
    event::EffectRef,
    ids::*,
    state::{mon_flags, scratch::*},
};
fn fixture(ability: &str, item: &str) -> Battle {
    let item = if item == "-" { "" } else { item };
    let p1 = format!("Pikachu||{item}|{ability}|thunderbolt,uturn|Serious||M|||100|,,,,,Electric");
    let p2 = "Pikachu|||static|thunderbolt|Serious||M|||100|,,,,,Electric";
    let mut b = Battle::new([1, 2, 3, 4], &p1, p2).unwrap();
    b.state.sides[0].active = [MonId(0), MonId::NONE];
    b.state.sides[1].active = [MonId(6), MonId::NONE];
    b.state.pokemon[0].flags |= mon_flags::ACTIVE;
    b.state.pokemon[6].flags |= mon_flags::ACTIVE;
    b
}
fn stat(key: &str) -> Stat {
    match key {
        "atk" => Stat::Atk,
        "def" => Stat::Def,
        "spa" => Stat::SpA,
        "spd" => Stat::SpD,
        "spe" => Stat::Spe,
        _ => panic!("unknown stat"),
    }
}
fn float(key: &str) -> f64 {
    match key {
        "NaN" => f64::NAN,
        "Infinity" => f64::INFINITY,
        _ => key.parse().unwrap(),
    }
}
fn seed(key: &str) -> [u16; 4] {
    key.split(',')
        .map(|n| n.parse().unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}
fn room(b: &mut Battle) {
    let cell = b
        .state
        .effects
        .alloc(Holder::FIELD, Holder::FIELD, dex::CONDITION_TRICKROOM, 0);
    b.state.field.pseudo_weather.push(cell);
}
fn active() -> ActiveMove {
    let m = dex::move_data(dex::MOVE_UTURN);
    ActiveMove {
        flags: m.flags,
        traits: m.traits,
        runtime_flags: 0,
        total_damage: 0,
        hit_targets: [MonId::NONE; 4],
        hit_target_len: 0,
        id: m.id,
        source_effect: EffectRef::None,
        type_changer_boosted: EffectId::NONE,
        ruined_stats: [MonId::NONE; 4],
        base_power: m.base_power as f64,
        accuracy: MoveAccuracy::Percent(100.0),
        hit_data: [HitData::default(); 12],
        self_boosts: OrderedBoosts::default(),
        effects: MoveEffectsScratch {
            base: &m.effects,
            boosts: OrderedBoosts::default(),
            status: m.effects.status,
            volatile_status: m.effects.volatile_status,
            side_condition: m.effects.side_condition,
            slot_condition: m.effects.slot_condition,
            weather: m.effects.weather,
            terrain: m.effects.terrain,
            pseudo_weather: m.effects.pseudo_weather,
            chance: None,
            heal: m.effects.heal,
            force_switch: m.effects.force_switch,
            self_switch: m.effects.self_switch,
            suppressed_hooks: 0,
        },
        self_effect: None,
        secondaries: [None; 4],
        secondary_count: 0,
        secondaries_present: false,
        recoil: m.recoil,
        drain: m.drain,
        damage: m.damage,
        self_destruct: m.self_destruct,
        offensive_stat: m.offensive_stat,
        defensive_stat: m.defensive_stat,
        offensive_target: m.offensive_target,
        ignore_immunity_types: 0,
        move_type: m.move_type,
        category: Category::Physical,
        target: m.target,
        priority: m.priority,
        crit_ratio: m.crit_ratio,
        hit: 0,
        multihit: m.multihit,
    }
}
#[test]
fn real_pinned_stat_weight_speed_and_best_stat_vectors() {
    let mut count = 0;
    for row in include_str!("vectors.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let c: Vec<_> = row.split('\t').collect();
        let flag = |i: usize| c[i] == "1";
        let mut b = match c[0] {
            "G" => fixture(c[1], c[2]),
            "B" => fixture(
                if flag(2) { "hugepower" } else { "static" },
                if flag(2) { "choiceband" } else { "" },
            ),
            "S" => fixture("static", if flag(3) { "choicescarf" } else { "" }),
            "W" => fixture(if flag(2) { "heavymetal" } else { "static" }, ""),
            _ => fixture("static", ""),
        };
        match c[0] {
            "C" => {
                b.state.pokemon[0].stored_stats = [111, 222, 333, 444, 555];
                let modifier = if c[3] == "undefined" {
                    None
                } else {
                    Some(float(c[3]))
                };
                let actual = b.calculate_stat(
                    MonId(0),
                    stat(c[1]),
                    c[2].parse().unwrap(),
                    modifier,
                    if flag(4) { Some(MonId(6)) } else { None },
                );
                assert_eq!(actual, float(c[5]), "{row}");
                assert_eq!(b.seed(), seed(c[6]), "{row}");
            }
            "G" => {
                b.state.pokemon[0].stored_stats = [111, 222, 333, 444, 555];
                b.state.pokemon[0].boosts = [c[4].parse().unwrap(); 7];
                if !flag(3) {
                    b.state.pokemon[0].flags &= !mon_flags::ACTIVE;
                }
                let stages = b.state.pokemon[0].boosts;
                let actual = b.get_stat(
                    MonId(0),
                    stat(c[5]),
                    StatOptions {
                        unboosted: flag(6),
                        unmodified: flag(7),
                    },
                );
                assert_eq!(actual, float(c[8]), "{row}");
                assert_eq!(b.seed(), seed(c[9]), "{row}");
                assert_eq!(
                    b.state.pokemon[0].boosts, stages,
                    "getStat mutated Pokemon boosts: {row}"
                );
            }
            "S" => {
                b.state.pokemon[0].stored_stats[4] = c[1].parse().unwrap();
                b.state.pokemon[0].boosts[4] = c[2].parse().unwrap();
                if flag(4) {
                    room(&mut b);
                }
                assert_eq!(
                    b.get_stat(MonId(0), Stat::Spe, StatOptions::default()),
                    float(c[5]),
                    "{row}"
                );
                assert_eq!(
                    b.pokemon_action_speed(MonId(0)),
                    c[6].parse::<u16>().unwrap(),
                    "{row}"
                );
                b.update_pokemon_speed(MonId(0));
                assert_eq!(
                    b.state.pokemon[0].speed,
                    c[7].parse::<u16>().unwrap(),
                    "{row}"
                );
                assert_eq!(b.seed(), seed(c[8]), "{row}");
            }
            "B" => {
                b.state.pokemon[0].stored_stats = c[1]
                    .split(',')
                    .map(|n| n.parse().unwrap())
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap();
                assert_eq!(
                    b.get_best_stat(
                        MonId(0),
                        StatOptions {
                            unboosted: flag(3),
                            unmodified: flag(4)
                        }
                    ),
                    stat(c[5]),
                    "{row}"
                );
                // Match the real event order and seed. Source repeats getStat
                // for each new maximum; these hooks have distinct priorities.
                assert_eq!(b.seed(), seed(c[6]), "{row}");
            }
            "W" => {
                b.state.pokemon[0].weighthg = c[1].parse().unwrap();
                if !flag(3) {
                    b.state.pokemon[0].flags &= !mon_flags::ACTIVE;
                }
                assert_eq!(b.get_weight(MonId(0)), float(c[4]), "{row}");
                assert_eq!(b.seed(), seed(c[5]), "{row}");
            }
            "H" => {
                b.scratch.moves[0] = Some(active());
                let first = b.move_hit_data(MonId(0), MoveHandle(0));
                assert!(first.present);
                assert!(!first.crit);
                assert_eq!(first.type_mod, 0);
                assert!(!first.bypass_protect);
                first.crit = true;
                first.type_mod = -2;
                first.bypass_protect = true;
                let same = b.move_hit_data(MonId(0), MoveHandle(0));
                assert_eq!(same.type_mod, -2);
                b.state.pokemon[0].position = 1;
                let next = *b.move_hit_data(MonId(0), MoveHandle(0));
                b.state.pokemon[0].position = 0;
                let restored = *b.move_hit_data(MonId(0), MoveHandle(0));
                let other = *b.move_hit_data(MonId(6), MoveHandle(0));
                assert_eq!(restored.crit, flag(5), "{row}");
                assert_eq!(restored.type_mod, c[6].parse::<i8>().unwrap(), "{row}");
                assert_eq!(restored.bypass_protect, flag(7), "{row}");
                assert_eq!(next.crit, flag(8), "{row}");
                assert_eq!(next.type_mod, c[9].parse::<i8>().unwrap(), "{row}");
                assert_eq!(next.bypass_protect, flag(10), "{row}");
                assert!(!other.crit);
                assert!(!other.bypass_protect);
                assert_eq!(other.type_mod, 0);
                assert_eq!(
                    b.scratch.moves[0]
                        .as_ref()
                        .unwrap()
                        .hit_data
                        .iter()
                        .filter(|d| d.present)
                        .count(),
                    3
                );
                assert_eq!(b.seed(), seed(c[12]), "{row}");
            }
            _ => panic!("unknown oracle row"),
        }
        assert_eq!(b.scratch.boosts_used, 0, "leaked boost scratch: {row}");
        assert_eq!(b.scratch.event_depth, 0, "leaked event frame: {row}");
        assert_eq!(b.scratch.handler_depth, 0, "leaked handler buffer: {row}");
        assert_eq!(
            b.scratch.unsent_lines, 4,
            "unexpected protocol output: {row}"
        );
        count += 1;
    }
    assert_eq!(count, 2277);
}
#[test]
fn per_slot_hit_metadata_follows_replacement_identity_in_the_same_move() {
    let mut b = fixture("static", "");
    b.scratch.moves[0] = Some(active());
    b.state.pokemon[1].position = 0;
    b.move_hit_data(MonId(0), MoveHandle(0)).type_mod = 2;
    // A replacement in p1a observes the existing p1a object; its stable MonId
    // must not allocate a second hit-data cell.
    assert_eq!(b.move_hit_data(MonId(1), MoveHandle(0)).type_mod, 2);
    b.scratch.moves[1] = Some(active());
    assert_eq!(b.move_hit_data(MonId(1), MoveHandle(1)).type_mod, 0);
}
#[test]
fn missing_boost_key_preserves_source_nan_and_clamps_extreme_stages() {
    assert!(super::boosted_stat(100.0, None).is_nan());
    assert_eq!(super::boosted_stat(100.0, Some(i8::MIN)), 25.0);
    assert_eq!(super::boosted_stat(100.0, Some(i8::MAX)), 400.0);
    let mut b = fixture("static", "");
    // The actual no-handler ModifyBoost call keeps a sparse, explicitly-zero
    // property; the full getStat input keeps all seven properties.
    let sparse = OrderedBoosts {
        values: [0; 7],
        order: [3, 0, 0, 0, 0, 0, 0],
        len: 1,
        present: 1 << 3,
    };
    let actual = b.stats_modify_boosts(MonId(0), sparse);
    assert_eq!(actual.len, 1);
    assert_eq!(actual.order[0], 3);
    assert_eq!(actual.present, 1 << 3);
    assert_eq!(actual.values, [0; 7]);
    assert_eq!(b.scratch.boosts_used, 0);
}
