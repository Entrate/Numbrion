//! getTarget / getRandomTarget / validTargetLoc / location helpers / getSmartTargets against
//! 43k vectors recorded from the real pinned Showdown (including the PRNG state after each call).
use super::fixture::*;
use crate::{
    actions::MoveInput,
    dex::{self, MoveTarget},
    ids::*,
    state::mon_flags,
};

fn mon_index(m: Option<MonId>) -> String {
    match m {
        Some(m) => ACTIVES.iter().position(|a| *a == m).expect("active").to_string(),
        None => "-".into(),
    }
}

fn move_target(name: &str) -> MoveTarget {
    match name {
        "normal" => MoveTarget::Normal,
        "self" => MoveTarget::SelfTarget,
        "any" => MoveTarget::Any,
        "allAdjacentFoes" => MoveTarget::AllAdjacentFoes,
        "allySide" => MoveTarget::AllySide,
        "allAdjacent" => MoveTarget::AllAdjacent,
        "adjacentAlly" => MoveTarget::AdjacentAlly,
        "all" => MoveTarget::All,
        "allies" => MoveTarget::Allies,
        "foeSide" => MoveTarget::FoeSide,
        "randomNormal" => MoveTarget::RandomNormal,
        other => panic!("unknown move target {other}"),
    }
}

#[test]
fn targeting_helpers_match_pinned_showdown() {
    let mut counts = [0usize; 6];
    for line in include_str!("targeting-vectors.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<&str> = line.split('\t').collect();
        let mut state = [0u8; 4];
        for (i, ch) in c[1].chars().enumerate() {
            state[i] = ch.to_digit(10).unwrap() as u8;
        }
        let user = ACTIVES[c[2].parse::<usize>().unwrap()];
        let mut b = battle_with(state);
        reseed(&mut b);
        let (result, seed_after): (String, String);
        match c[0] {
            "T" => {
                let loc: i8 = c[4].parse().unwrap();
                let original = match c[5] {
                    "-" => None,
                    n => Some(ACTIVES[n.parse::<usize>().unwrap()]),
                };
                if c[6] == "1" {
                    b.state.pokemon[user.0 as usize].ability = dex::ABILITY_STALWART;
                }
                if c[7] == "1" {
                    let cell = b.state.effects.alloc(
                        Holder::mon(user),
                        Holder::mon(user),
                        dex::CONDITION_TWOTURNMOVE,
                        0,
                    );
                    b.state.pokemon[user.0 as usize].volatiles.push(cell);
                }
                let r = b.get_target(user, MoveInput::Dex(move_id(c[3])), Some(loc), original);
                result = mon_index(r);
                seed_after = seed_string(&b);
                counts[0] += 1;
            }
            "R" => {
                let r = b.get_random_target(user, MoveInput::Dex(move_id(c[3])));
                result = mon_index(r);
                seed_after = seed_string(&b);
                counts[1] += 1;
            }
            "S" => {
                let target = ACTIVES[c[4].parse::<usize>().unwrap()];
                let h = b.get_active_move(MoveInput::Dex(dex::MOVE_DRAGONDARTS));
                let list = b.get_smart_targets(user, target, h);
                let ids: Vec<String> = (0..list.len as usize)
                    .map(|i| match list.entries[i] {
                        crate::actions::HitTarget::Pokemon(m) => mon_index(Some(m)),
                        other => panic!("{other:?}"),
                    })
                    .collect();
                let smart = b.input_smart_target(MoveInput::Active(h));
                result = format!("{}/{}", ids.join(","), u8::from(smart));
                seed_after = seed_string(&b);
                counts[2] += 1;
            }
            "L" => {
                let target = ACTIVES[c[4].parse::<usize>().unwrap()];
                result = b.get_loc_of(user, target).to_string();
                seed_after = seed_string(&b);
                counts[3] += 1;
            }
            "A" => {
                let loc: i8 = c[4].parse().unwrap();
                result = mon_index(b.get_at_loc(user, loc));
                seed_after = seed_string(&b);
                counts[4] += 1;
            }
            "V" => {
                let loc: i8 = c[4].parse().unwrap();
                result = u8::from(b.valid_target_loc(loc, user, move_target(c[3]))).to_string();
                seed_after = seed_string(&b);
                counts[5] += 1;
            }
            other => panic!("unknown vector kind {other}"),
        }
        assert_eq!(result, c[8], "{line}");
        assert_eq!(seed_after, c[9], "{line}");
    }
    assert_eq!(counts.iter().sum::<usize>(), 43208, "{counts:?}");
    let _ = mon_flags::ACTIVE;
}
