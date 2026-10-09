#[allow(dead_code, unused_imports)]
#[path = "../../../effects/abilities/passiveoffense/support.rs"]
mod setup;
use crate::{
    actions::{HitTarget, Targets},
    dex::MoveTarget,
    ids::*,
    state::mon_flags,
};
fn ids(targets: Targets) -> Vec<u8> {
    targets.entries[..targets.len as usize]
        .iter()
        .map(|t| match t {
            HitTarget::Pokemon(m) => m.0,
            _ => panic!("non-Pokemon query result"),
        })
        .collect()
}
#[test]
fn ally_foe_queries_match_pinned_source_for_hp_flags_and_missing_slots() {
    let mut b = setup::battle();
    let seed = b.seed();
    let log = b.scratch.unsent_lines;
    let mut count = 0;
    for line in include_str!("vectors.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line.split('\t').collect();
        let mask = c[0].parse::<u32>().unwrap();
        let user = MonId(c[1].parse().unwrap());
        for (i, m) in [0, 1, 6, 7].into_iter().enumerate() {
            b.state.pokemon[m].hp = if mask & (1 << i) != 0 { 100 } else { 0 };
            b.state.pokemon[m].flags = if mask & (1 << (i + 4)) != 0 {
                mon_flags::FAINTED
            } else {
                0
            };
        }
        b.state.sides[0].active = [MonId(0), if mask % 3 == 0 { MonId::NONE } else { MonId(1) }];
        b.state.sides[1].active = [MonId(6), if mask % 5 == 0 { MonId::NONE } else { MonId(7) }];
        let result = match c[2] {
            "alliesAndSelf" => b.allies_and_self(user),
            "allies" => b.allies(user),
            "adjacentAllies" => b.adjacent_allies(user),
            "foes" => b.foes(user, false),
            "foesAll" => b.foes(user, true),
            "adjacentFoes" => b.adjacent_foes(user),
            _ => panic!("unknown query"),
        };
        let expected: Vec<u8> = if c[3].is_empty() {
            vec![]
        } else {
            c[3].split(',').map(|n| n.parse().unwrap()).collect()
        };
        assert_eq!(ids(result), expected, "{line}");
        assert_eq!(b.seed(), seed);
        assert_eq!(b.scratch.unsent_lines, log);
        count += 1;
    }
    assert_eq!(count, 3072);
}
#[test]
fn scoped_choosable_target_kinds_match_source() {
    let b = setup::battle();
    for (t, want) in [
        (MoveTarget::Normal, true),
        (MoveTarget::Any, true),
        (MoveTarget::AdjacentAlly, true),
        (MoveTarget::SelfTarget, false),
        (MoveTarget::AllAdjacentFoes, false),
        (MoveTarget::AllAdjacent, false),
        (MoveTarget::All, false),
        (MoveTarget::Allies, false),
        (MoveTarget::AllySide, false),
        (MoveTarget::FoeSide, false),
        (MoveTarget::RandomNormal, false),
    ] {
        assert_eq!(b.target_type_choices(t), want);
    }
}
