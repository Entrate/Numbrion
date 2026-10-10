use crate::{
    Battle,
    actions::MoveHandle,
    dex::{self, Category},
    event::{EffectRef, Relay},
    ids::*,
    state::{mon_flags, scratch::*},
};
fn fixture() -> Battle {
    let packed = "Pikachu||lightball|static|thunderbolt,uturn|Serious||M|||100|,,,,,Electric";
    let mut b = Battle::new([1, 2, 3, 4], packed, packed).unwrap();
    b.state.sides[0].active = [MonId(0), MonId::NONE];
    b.state.sides[1].active = [MonId(6), MonId::NONE];
    b.state.pokemon[0].flags |= mon_flags::ACTIVE;
    b.state.pokemon[6].flags |= mon_flags::ACTIVE;
    b
}
fn id(kind: EffectKind, key: &str) -> EffectId {
    if key == "-" {
        EffectId::NONE
    } else {
        dex::lookup(kind, key).unwrap()
    }
}
fn types(csv: &str) -> Vec<TypeId> {
    if csv == "-" {
        vec![]
    } else {
        csv.split(',').map(|s| dex::type_id(s).unwrap()).collect()
    }
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
        category: m.category,
        target: m.target,
        priority: m.priority,
        crit_ratio: m.crit_ratio,
        hit: 0,
        multihit: m.multihit,
    }
}
#[test]
fn scoped_query_contact_type_and_raw_helpers_match_real_pinned_oracle() {
    let mut count = 0;
    for row in include_str!("vectors.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let c: Vec<_> = row.split('\t').collect();
        let mut b = fixture();
        let flag = |i: usize| c[i] == "1";
        match c[0] {
            "Q" => {
                let ability = id(EffectKind::Ability, c[1]);
                let item = id(EffectKind::Item, c[2]);
                let p = &mut b.state.pokemon[0];
                p.ability = ability;
                p.item = item;
                if !flag(3) {
                    p.flags &= !mon_flags::ACTIVE;
                }
                if flag(4) {
                    p.flags |= mon_flags::TRANSFORMED;
                }
                assert_eq!(b.ignoring_ability(MonId(0)), flag(5), "{row}");
                assert_eq!(b.ignoring_item(MonId(0)), flag(6), "{row}");
                assert_eq!(b.has_ability(MonId(0), &[ability]), flag(7), "{row}");
                assert_eq!(b.has_item(MonId(0), &[item]), flag(8), "{row}");
                assert!(!b.has_ability(MonId(0), &[]));
                assert!(!b.has_item(MonId(0), &[]));
            }
            "T" => {
                let base = types(c[1]);
                let p = &mut b.state.pokemon[0];
                p.types = [TypeId::NONE; 2];
                p.types[..base.len()].copy_from_slice(&base);
                p.added_type = types(c[2]).first().copied().unwrap_or(TypeId::NONE);
                p.terastallized = types(c[3]).first().copied().unwrap_or(TypeId::NONE);
                let actual = b.get_types(MonId(0), flag(4), flag(5));
                assert_eq!(&actual.values[..actual.len as usize], types(c[6]), "{row}");
                assert_eq!(
                    b.state.pokemon[0]
                        .types
                        .iter()
                        .copied()
                        .filter(|&t| t != TypeId::NONE)
                        .collect::<Vec<_>>(),
                    types(c[7]),
                    "{row}"
                );
                assert!(
                    b.scratch.types.iter().all(Option::is_none),
                    "type relay leak: {row}"
                );
            }
            "C" => {
                let mut mv = active();
                mv.flags = 0;
                if flag(1) {
                    mv.flags |= dex::FLAG_CONTACT;
                }
                if flag(2) {
                    mv.flags |= dex::FLAG_PROTECT;
                }
                mv.category = if c[3] == "Status" {
                    Category::Status
                } else {
                    Category::Physical
                };
                b.scratch.moves[0] = Some(mv);
                assert_eq!(
                    b.check_move_makes_contact(MoveHandle(0), MonId(0), MonId(6), true),
                    flag(5),
                    "{row}"
                );
                assert_eq!(
                    b.check_move_bypasses_protect(MoveHandle(0), MonId(0), MonId(6), flag(4)),
                    flag(6),
                    "{row}"
                );
                assert_eq!(b.scratch.unsent_lines, 4, "{row}");
            }
            "B" => {
                let stages: Vec<i8> = c[1].split(',').map(|s| s.parse().unwrap()).collect();
                b.state.pokemon[0].boosts.copy_from_slice(&stages);
                let input = OrderedBoosts {
                    values: [3, -2, -3, 0, 0, 1, -1],
                    order: [4, 0, 6, 1, 5, 2, 0],
                    len: 6,
                    present: 0b1110111,
                };
                let actual = b.get_capped_boost(MonId(0), input);
                let names = ["atk", "def", "spa", "spd", "spe", "accuracy", "evasion"];
                assert_eq!(
                    actual.order[..actual.len as usize]
                        .iter()
                        .map(|&i| names[i as usize])
                        .collect::<Vec<_>>()
                        .join(","),
                    c[2],
                    "{row}"
                );
                assert_eq!(
                    actual.order[..actual.len as usize]
                        .iter()
                        .map(|&i| actual.values[i as usize].to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                    c[3],
                    "{row}"
                );
                assert_eq!(
                    b.positive_boosts(MonId(0)),
                    c[4].parse::<u8>().unwrap(),
                    "{row}"
                );
                assert_eq!(b.state.pokemon[0].boosts, stages.as_slice());
            }
            "G" => {
                b.state.pokemon[0].ability = id(EffectKind::Ability, c[1]);
                if !flag(2) {
                    b.state.pokemon[0].flags &= !mon_flags::ACTIVE;
                }
                b.state.pokemon[0].types =
                    [if flag(3) { TypeId(3) } else { TypeId(13) }, TypeId::NONE];
                let mut mv = active();
                if flag(4) {
                    mv.runtime_flags |= move_runtime::IGNORE_ABILITY;
                }
                b.scratch.moves[0] = Some(mv);
                b.set_active_move(
                    Some(MoveHandle(0)),
                    Some(if flag(5) { MonId(0) } else { MonId(6) }),
                    Some(MonId(0)),
                );
                let expected = match c[7] {
                    "null" => Relay::Null,
                    "1" => Relay::Bool(true),
                    "0" => Relay::Bool(false),
                    _ => panic!("{row}"),
                };
                assert_eq!(b.is_grounded(MonId(0), flag(6)), expected, "{row}");
            }
            "W" => {
                let weather = id(EffectKind::Condition, c[1]);
                let weather_cell = b.state.field.weather;
                b.state.effects.cells[weather_cell.0 as usize].id = weather;
                let p = &mut b.state.pokemon[6];
                p.ability = if flag(2) {
                    dex::ABILITY_CLOUDNINE
                } else {
                    dex::ABILITY_STATIC
                };
                if !flag(3) {
                    p.flags &= !mon_flags::ACTIVE;
                }
                if flag(4) {
                    p.flags |= mon_flags::FAINTED;
                }
                if flag(5) {
                    b.state.effects.cells[p.ability_state.0 as usize].present |=
                        crate::state::present::ENDING;
                }
                assert_eq!(
                    b.effective_weather(MonId(0)),
                    id(EffectKind::Condition, c[6]),
                    "{row}"
                );
            }
            "H" => {
                b.state.pokemon[0].hp = 50;
                let amount = match c[1] {
                    "undefined" => None,
                    "NaN" => Some(f64::NAN),
                    "Infinity" => Some(f64::INFINITY),
                    n => Some(n.parse().unwrap()),
                };
                let expected = if c[2] == "Infinity" {
                    f64::INFINITY
                } else {
                    c[2].parse().unwrap()
                };
                assert_eq!(b.get_undynamaxed_hp(MonId(0), amount), expected, "{row}");
            }
            _ => panic!("unrecognized oracle row: {row}"),
        }
        assert_eq!(b.seed(), [1, 2, 3, 4], "{row}");
        count += 1;
    }
    assert_eq!(count, 1196);
}
#[test]
fn move_lookup_tracks_transform_slots_and_presence_not_pp_or_disabling() {
    let mut b = fixture();
    assert_eq!(
        b.has_move(MonId(0), dex::MOVE_UTURN),
        Relay::Move(dex::MOVE_UTURN)
    );
    assert_eq!(
        b.has_move(MonId(0), dex::MOVE_FLAMETHROWER),
        Relay::Bool(false)
    );
    b.state.pokemon[0].base_move_slots[0].pp = 0;
    b.state.pokemon[0].base_move_slots[0].flags = 0xff;
    assert!(b.get_move_data(MonId(0), dex::MOVE_THUNDERBOLT).is_some());
    let p = &mut b.state.pokemon[0];
    p.flags |= mon_flags::TRANSFORMED;
    p.virtual_move_slots[0] = p.base_move_slots[1];
    p.virtual_move_count = 1;
    assert_eq!(
        b.has_move(MonId(0), dex::MOVE_THUNDERBOLT),
        Relay::Bool(false)
    );
    assert_eq!(
        b.get_move_data(MonId(0), dex::MOVE_UTURN).unwrap().id,
        dex::MOVE_UTURN
    );
    assert_eq!(b.get_nature(MonId(0)).unwrap().key, "serious");
}
#[test]
fn named_format_excluded_effects_never_match_none_and_semi_invulnerability_reads_conditions() {
    let mut b = fixture();
    b.state.pokemon[0].ability = EffectId::NONE;
    b.state.pokemon[0].item = EffectId::NONE;
    assert!(!b.query_has_item(MonId(0), dex::key_ids!("abilityshield")));
    assert!(!b.query_has_ability(MonId(0), dex::key_ids!("klutz")));
    assert!(!b.is_semi_invulnerable(MonId(0)));
    let cell = b.state.effects.alloc(
        Holder::mon(MonId(0)),
        Holder::mon(MonId(0)),
        dex::CONDITION_PHANTOMFORCE,
        0,
    );
    b.state.pokemon[0].volatiles.push(cell);
    assert!(b.is_semi_invulnerable(MonId(0)));
    b.state.pokemon[0].volatiles.remove(0);
    assert!(!b.is_semi_invulnerable(MonId(0)));
    b.state.pokemon[0].terastallized = dex::type_id("Flying").unwrap();
    assert_eq!(b.is_grounded(MonId(0), false), Relay::Bool(false));
    assert_eq!(b.is_grounded(MonId(0), true), Relay::Bool(true));
    assert!(b.has_type(MonId(0), &[dex::type_id("Flying").unwrap()]));
}
