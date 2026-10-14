//! Replay gen9randomdoublesbattle-2697901418, Numbrion = p1. Turn 1's Psychic precedes
//! Squawkabilly's Brave Bird, which requires Scarf Munkidori. Ordinary sampling with replacement
//! can miss its 366/2823 pool entries throughout a collapsed rebuild and fail at compared line 12.
//! The pool fixture retains every opponent variant and its live pool-s1-200k frequency, plus normal
//! pool alternatives including Illusion. Teams match the replay's reveals and exact own HP.
//! The unrevealed sixth p1 member is inert Magmar filler. Unrevealed moves and Tera types are chosen
//! from supported sets. Damage changes replacement timing, so simulation choices have fallbacks.
use super::*;
use engine::{dex, ids::EffectKind};

fn fixture() -> ([Arc<TeamDef>; 2], Arc<SetPool>) {
    let teams: Vec<_> = include_str!("../../testdata/real-golgan.txt")
        .lines()
        .map(|s| Arc::new(TeamDef::unpack(s).unwrap()))
        .collect();
    let mut sets = SetPool::new();
    for team in &teams {
        for set in &team.sets[..team.len as usize] {
            sets.add_set(set);
        }
    }
    for line in include_str!("../../testdata/real-golgan-pool.txt").lines() {
        let (count, packed) = line.split_once('\t').unwrap();
        let team = TeamDef::unpack(packed).unwrap();
        for _ in 0..count.parse::<usize>().unwrap() {
            sets.add_set(&team.sets[0]);
        }
    }
    let pool = common::test_pool();
    for i in 0..pool.len() {
        sets.add_team(pool.get(i)).unwrap();
    }
    ([teams[0].clone(), teams[1].clone()], Arc::new(sets))
}

fn plan(side: u8, turn: u32) -> [&'static str; 2] {
    match (side, turn) {
        (0, 1) => ["Toxic 1", "Brave Bird 2"],
        (0, 2) => ["switch Calyrex", "Brave Bird 1"],
        (0, 3) => ["Psychic 1", "Quick Attack 2"],
        (0, 4) => ["Leech Seed 2", "Protect 0"],
        (0, 5) => ["Leech Seed 2", "Close Combat 1"],
        (0, 6) => ["Leech Seed 2", "Iron Defense 0 tera"],
        (0, 7) => ["Leaf Storm 2", "Body Press 1"],
        (0, _) => ["Psychic 2", "Body Press 1"],
        (1, 1) => ["switch Okidogi", "Psychic 1"],
        (1, 2) => ["Gunk Shot 2", "Psychic 1"],
        (1, 3) => ["Electroweb 0", "switch Blissey"],
        (1, 4..=5) => ["Thunderbolt 2", "Helping Hand -1"],
        (1, 6) => ["Tera Blast 1 tera", "switch Munkidori"],
        (1, 7) => ["Tera Blast 1", "Water Spout 0"],
        (1, _) => ["Thunder Wave 2", "Icicle Spear 1"],
        _ => unreachable!(),
    }
}

fn choose(b: &B, side: SideId) -> Vec<SlotChoice> {
    let forced = b.side_request_kind(side) == RequestKind::Switch;
    let desired = plan(side.0, b.turn());
    let replacement = match (side.0, b.turn()) {
        (0, 0..=3) => "Lycanroc",
        (0, _) => "Bastiodon",
        (1, 0..=2) => "Regieleki",
        (1, 3..=6) => "Kyogre",
        (1, _) => "Blissey",
        _ => unreachable!(),
    };
    let legal = b.legal_actions(side);
    let party = b.state.sides[side.0 as usize].party;
    let mut choices: Vec<SlotChoice> = vec![];
    for (slot, ls) in legal.slots[..legal.slot_count as usize].iter().enumerate() {
        if ls.automatic_pass {
            choices.push(SlotChoice::default());
            continue;
        }
        if let Some(name) = if forced {
            Some(replacement)
        } else {
            desired[slot].strip_prefix("switch ")
        } {
            let species = dex::lookup(EffectKind::Species, name).unwrap();
            let free = |p: &usize| {
                ls.switch_mask & (1 << p) != 0
                    && !choices
                        .iter()
                        .any(|c| c.kind == ChoiceKind::Switch && c.switch_to == party[*p])
            };
            if let Some(p) = (0..6)
                .filter(free)
                .find(|&p| b.state.pokemon[party[p].0 as usize].species == species)
                .or_else(|| (0..6).find(free))
            {
                choices.push(SlotChoice {
                    kind: ChoiceKind::Switch,
                    switch_to: party[p],
                    ..SlotChoice::default()
                });
                continue;
            }
        }
        let tera = desired[slot].ends_with(" tera");
        let (name, target) = desired[slot]
            .trim_end_matches(" tera")
            .rsplit_once(' ')
            .unwrap();
        let wanted = dex::lookup(EffectKind::Move, name);
        let moves = &ls.moves[..ls.move_count as usize];
        let m = moves
            .iter()
            .find(|m| Some(m.move_id) == wanted)
            .unwrap_or(&moves[0]);
        let target = target.parse::<i8>().unwrap_or(0);
        let target = if m.targets.contains(target) {
            target
        } else {
            [0, 1, 2, -1, -2]
                .into_iter()
                .find(|&t| m.targets.contains(t))
                .unwrap()
        };
        choices.push(SlotChoice {
            kind: ChoiceKind::Move,
            move_slot: m.move_slot,
            move_id: m.move_id,
            move_kind: m.move_kind,
            target_loc: target,
            tera: tera && m.can_terastallize,
            ..SlotChoice::default()
        });
    }
    assert!(
        b.is_legal_joint_choice(side, &choices),
        "turn {} p{}: {choices:?}",
        b.turn(),
        side.0 + 1
    );
    choices
}

#[test]
fn real_golgan_worlds_at_every_decision() {
    let (teams, sets) = fixture();
    for (builder_seed, seed) in
        (0..8).flat_map(|builder_seed| [1, 2, 3, 7].map(|seed| (builder_seed, seed)))
    {
        // Keep the full simulation for seed 1. The other damage seeds exercise the turn-1
        // Scarf inference; later HP-bucket exhaustion is a separate bounded-search concern.
        let last_turn = if seed == 1 { 8 } else { 2 };
        let mut b = B::from_team_defs(
            [1, seed, 3, 1],
            teams[0].clone(),
            teams[1].clone(),
            names(),
            TextLog::default(),
        )
        .unwrap();
        b.start().unwrap();
        let viewer = SideId(0);
        let mut w = WorldBuilder::new(
            viewer,
            b.teams().sides[0].clone(),
            names(),
            sets.clone(),
            WorldConfig::default(),
            builder_seed,
        )
        .unwrap();
        w.push_lines(
            player_view(&b.log.entries, 0, viewer)
                .iter()
                .map(String::as_str),
        );
        let mut cursor = b.log.entries.len();
        let mut checked = 0;
        while !b.state.ended && b.turn() <= last_turn {
            for side in [viewer, SideId(1)] {
                if !needs(&b, side) {
                    continue;
                }
                let choice = choose(&b, side);
                if side == viewer {
                    checked += 1;
                    w.push_request(&b.request_json(0).unwrap());
                    let worlds = w.worlds::<TextLog>(8, b.turn() as u64).unwrap_or_else(|e| {
                        panic!(
                            "builder seed {builder_seed}, battle seed {seed}, turn {}: {e}; {}",
                            b.turn(),
                            w.last_failure()
                        )
                    });
                    for world in worlds {
                        assert_eq!(world.request_json(0), b.request_json(0));
                    }
                    w.push_choice(&choice_text(&b, side, &choice));
                }
                b.choose_typed(side, &choice).unwrap();
            }
            let lines = player_view(&b.log.entries, cursor, viewer);
            w.push_lines(lines.iter().map(String::as_str));
            cursor = b.log.entries.len();
        }
        assert!(
            checked >= last_turn,
            "only {checked} decisions for battle seed {seed}"
        );
    }
}

#[test]
fn actual_golgan_protocol() {
    let (teams, sets) = fixture();
    let lines: Vec<_> = include_str!("../../testdata/real-golgan.log")
        .lines()
        .collect();
    let mut w = WorldBuilder::new(
        SideId(0),
        teams[0].clone(),
        ["Numbrion".into(), "GOLGAN".into()],
        sets,
        WorldConfig::default(),
        1,
    )
    .unwrap();
    let steps = [
        ("|turn|1", "move toxic 1, move bravebird 2"),
        ("|turn|2", "switch 3, move bravebird 1"),
        ("|turn|3", "move psychic 1, move quickattack 2"),
        ("|switch|p1b: Lycanroc|", "pass, switch 4"),
        ("|turn|4", "move leechseed 2, move protect"),
        ("|turn|5", "move leechseed 2, move closecombat 1"),
        ("|switch|p1b: Bastiodon|", "pass, switch 5"),
        ("|turn|6", "move leechseed 2, move irondefense terastallize"),
        ("|turn|7", "move leafstorm 2, move bodypress 1"),
        ("|turn|8", ""),
    ];
    let mut cursor = 0;
    for (boundary, choice) in steps {
        let at = lines.iter().position(|l| l.starts_with(boundary)).unwrap();
        let end = if boundary.starts_with("|switch|") {
            at
        } else {
            at + 1
        };
        w.push_lines(lines[cursor..end].iter().copied());
        w.worlds::<TextLog>(8, 100)
            .unwrap_or_else(|e| panic!("boundary {boundary}: {e}; {}", w.last_failure()));
        if !choice.is_empty() {
            w.push_choice(choice);
        }
        cursor = end;
    }
}

#[test]
fn golgan_speed_order_repairs_choice_scarf_without_rebuilding() {
    let (teams, sets) = fixture();
    let mut repaired = 0;
    for builder_seed in 0..8 {
        let mut b = B::from_team_defs([1, 1, 3, 1], teams[0].clone(), teams[1].clone(), names(), TextLog::default()).unwrap();
        b.start().unwrap();
        let viewer = SideId(0);
        let config = WorldConfig { particles: 1, rebuild_budget: 1, fresh_per_update: 0, ..WorldConfig::default() };
        let mut w = WorldBuilder::new(viewer, b.teams().sides[0].clone(), names(), sets.clone(), config, builder_seed).unwrap();
        w.push_lines(player_view(&b.log.entries, 0, viewer).iter().map(String::as_str));
        w.push_request(&b.request_json(0).unwrap());
        w.update().unwrap();
        let initially_scarfed = w.opponent_teams()[0].sets[..6].iter()
            .find(|s| s.species == dex::SPECIES_MUNKIDORI).unwrap().item == dex::ITEM_CHOICESCARF;
        let builds = w.stats.builds;
        let cursor = b.log.entries.len();
        let own = choose(&b, viewer);
        let opp = choose(&b, SideId(1));
        w.push_choice(&choice_text(&b, viewer, &own));
        b.choose_typed(viewer, &own).unwrap();
        b.choose_typed(SideId(1), &opp).unwrap();
        w.push_lines(player_view(&b.log.entries, cursor, viewer).iter().map(String::as_str));
        w.push_request(&b.request_json(0).unwrap());
        let worlds = w.worlds::<TextLog>(1, 99).unwrap_or_else(|e| panic!("seed {builder_seed}: {e}; {}", w.last_failure()));
        assert_eq!(worlds[0].request_json(0), b.request_json(0));
        assert_eq!(w.stats.builds, builds);
        assert_eq!(w.stats.deaths, w.stats.set_repairs, "a dead world comes back only by set repair");
        assert!(w.opponent_teams()[0].sets[..6].iter().any(|s| s.species == dex::SPECIES_MUNKIDORI && s.item == dex::ITEM_CHOICESCARF));
        if !initially_scarfed {
            assert!(w.stats.set_repairs > 0);
            repaired += 1;
        }
    }
    assert!(repaired >= 4, "expected common non-Scarf sets to need repair, got {repaired}");
}
