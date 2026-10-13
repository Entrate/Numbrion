//! Regression from spectator replay gen9randomdoublesbattle-2697543270 (Numbrion = p1).
//! These are pool sets, with the replay's explicit genders. Goodra, Dugtrio and Fezandipiti
//! came from pool-s1-200k.txt.gz; the other sets came from pool-400.txt.gz. Both disguised
//! entries must spend Zoroark's PP, and the later real Bastiodon must remain a separate mon.
use super::*;
use engine::{
    dex,
    ids::{EffectId, EffectKind, MonId},
};

fn fixture() -> ([Arc<TeamDef>; 2], Arc<SetPool>) {
    let packed: Vec<_> = include_str!("../../testdata/real-zoroark.txt")
        .lines()
        .collect();
    let teams = packed
        .iter()
        .map(|s| Arc::new(TeamDef::unpack(s).unwrap()))
        .collect::<Vec<_>>();
    let mut sets = SetPool::new();
    for team in &teams {
        for set in &team.sets {
            sets.add_set(set);
        }
    }
    // Keep normal pool alternatives too: the builder receives sets, never the real opponent team.
    let pool = common::test_pool();
    for i in 0..pool.len() {
        sets.add_team(pool.get(i)).unwrap();
    }
    ([teams[0].clone(), teams[1].clone()], Arc::new(sets))
}

fn id(kind: EffectKind, name: &str) -> EffectId {
    dex::lookup(kind, name).unwrap()
}

/// Resolve the replay choice against the current legal actions. Differences in damage can
/// change replacement timing; prefer the replay's replacement, then the first legal reserve.
fn replay_choice(b: &B, side: SideId) -> Vec<SlotChoice> {
    let turn = b.turn();
    let switch = b.side_request_kind(side) == RequestKind::Switch;
    let desired: [&str; 2] = if switch {
        match (side.0, turn) {
            (0, 0..=4) => ["Dugtrio", "Dugtrio"],
            (0, 5) => ["Blaziken", "Blaziken"],
            (0, 6..=10) => ["Dugtrio", "Dugtrio"],
            (0, _) => ["Bellossom", "Bellossom"],
            (1, 0..=2) => ["Mewtwo", "Mewtwo"],
            (1, 3) => ["Zoroark", "Zoroark"],
            (1, 4..=8) => ["Goodra", "Goodra"],
            (1, _) => ["Bastiodon", "Bastiodon"],
            _ => unreachable!(),
        }
    } else {
        match (side.0, turn) {
            (0, 1) => ["Dazzling Gleam", "Close Combat"],
            (0, 2) => ["Flash Cannon", "Close Combat"],
            (0, 3) => ["Flash Cannon", "Bullet Seed"],
            (0, 4) => ["Rock Slide", "Bullet Seed"],
            (0, 5) => ["Protect", "Bullet Seed"],
            (0, 6) => ["switch Fezandipiti", "Protect"],
            (0, 7) => ["Gunk Shot", "Close Combat"],
            (0, 8) => ["Gunk Shot", "Protect"],
            (0, 9) => ["Icy Wind", "Stomping Tantrum"],
            (0, 10) => ["Roost", "Stomping Tantrum"],
            (0, _) => ["Moonblast", "Stomping Tantrum"],
            (1, 1) => ["switch Infernape", "Protect"],
            (1, 2) => ["Psystrike", "switch Grimmsnarl"],
            (1, 3) => ["Fire Blast", "Light Screen"],
            (1, 4) => ["Protect", "Reflect"],
            (1, 5) => ["Sludge Bomb", "Power Whip"],
            (1, 6) => ["Nasty Plot", "Power Whip"],
            (1, 7) => ["Sludge Bomb", "Thunderbolt"],
            (1, 8) => ["Dark Pulse", "Thunderbolt"],
            (1, 9) => ["Dark Pulse", "Thunderbolt"],
            (1, 10) => ["Iron Defense", "Scald"],
            (1, _) => ["Foul Play", "Thunderbolt"],
            _ => unreachable!(),
        }
    };
    let targets = match (side.0, turn) {
        (0, 2..=3) => [1, if turn == 2 { 2 } else { 1 }],
        (0, 4..=5) => [1, 2],
        (0, 7..=8) => [2, 1],
        (0, 9..=11) => [2, 1],
        (1, 2) => [2, 0],
        (1, 3) => [1, 0],
        (1, 5..=6) => [2, 1],
        (1, 7..=9) => [2, 1],
        (1, 10..=11) => [2, if turn == 10 { 2 } else { 1 }],
        _ => [0, 1],
    };
    let legal = b.legal_actions(side);
    let party = b.state.sides[side.0 as usize].party;
    let mut choices = Vec::new();
    for (slot, ls) in legal.slots[..legal.slot_count as usize].iter().enumerate() {
        if ls.automatic_pass {
            choices.push(SlotChoice::default());
            continue;
        }
        let want_switch = switch || desired[slot].starts_with("switch ");
        if want_switch {
            let species = id(
                EffectKind::Species,
                desired[slot].trim_start_matches("switch "),
            );
            let candidates = (0..6)
                .filter(|&p| ls.switch_mask & (1 << p) != 0)
                .filter(|&p| {
                    !choices.iter().any(|c: &SlotChoice| {
                        c.kind == ChoiceKind::Switch && c.switch_to == party[p]
                    })
                });
            let reserve = candidates
                .clone()
                .find(|&p| b.state.pokemon[party[p].0 as usize].species == species)
                .or_else(|| candidates.into_iter().next());
            if let Some(p) = reserve {
                choices.push(SlotChoice {
                    kind: ChoiceKind::Switch,
                    switch_to: party[p],
                    ..SlotChoice::default()
                });
                continue;
            }
        }
        let moves = &ls.moves[..ls.move_count as usize];
        let wanted = if want_switch {
            dex::MOVE_PROTECT
        } else {
            id(EffectKind::Move, desired[slot])
        };
        let m = moves
            .iter()
            .find(|m| m.move_id == wanted)
            .unwrap_or(&moves[0]);
        let target = if m.targets.contains(targets[slot]) {
            targets[slot]
        } else {
            [0, 1, 2, -1, -2]
                .into_iter()
                .find(|&loc| m.targets.contains(loc))
                .unwrap()
        };
        choices.push(SlotChoice {
            kind: ChoiceKind::Move,
            move_slot: m.move_slot,
            move_id: m.move_id,
            move_kind: m.move_kind,
            target_loc: target,
            tera: m.can_terastallize
                && ((side.0 == 1 && turn == 7 && slot == 0)
                    || (side.0 == 0 && turn == 10 && slot == 1)),
            ..SlotChoice::default()
        });
    }
    assert!(
        b.is_legal_joint_choice(side, &choices),
        "turn {turn} p{}: {choices:?}",
        side.0 + 1
    );
    choices
}

#[test]
fn real_zoroark_battle_worlds_at_every_decision() {
    let (teams, sets) = fixture();
    // Selected once from the replay: Fire Blast lands on turn 3, Illusion breaks on
    // turn 7, and Goodra survives until the real Bastiodon has entered p2a.
    let mut b = B::from_team_defs(
        [1, 2, 3, 1],
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
        sets,
        WorldConfig::default(),
        41,
    )
    .unwrap();
    w.push_lines(
        player_view(&b.log.entries, 0, viewer)
            .iter()
            .map(String::as_str),
    );
    let mut cursor = b.log.entries.len();
    let (mut checked, mut failed, mut missing_illusion, mut wrong_attribution) = (0, 0, 0, 0);
    'battle: while !b.state.ended && b.turn() <= 12 {
        for side in [viewer, SideId(1)] {
            if !needs(&b, side) {
                continue;
            }
            let choice = replay_choice(&b, side);
            if side == viewer {
                w.push_request(&b.request_json(0).unwrap());
                checked += 1;
                let lines = player_view(&b.log.entries, 0, viewer);
                let broken = lines
                    .iter()
                    .any(|s| s.starts_with("|replace|p2a: Zoroark|"));
                let evidence = lines
                    .iter()
                    .any(|s| s.starts_with("|move|p2b: Bastiodon|Protect|"));
                match w.worlds::<TextLog>(8, 100 + checked) {
                    Ok(worlds) => {
                        for world in &worlds {
                            assert_eq!(
                                world.request_json(0),
                                b.request_json(0),
                                "decision {checked}"
                            );
                        }
                        if evidence
                            && !broken
                            && !worlds.iter().any(|world| {
                                world.teams().sides[1]
                                    .sets
                                    .iter()
                                    .any(|s| s.ability == dex::ABILITY_ILLUSION)
                            })
                        {
                            missing_illusion += 1;
                        }
                        if broken {
                            let mut attributed = true;
                            for world in &worlds {
                                let z = world.teams().sides[1]
                                    .sets
                                    .iter()
                                    .position(|s| s.species == dex::SPECIES_ZOROARK)
                                    .unwrap();
                                let mon = MonId(6 + z as u8);
                                let real = &b.state.pokemon[7];
                                let p = &world.state.pokemon[mon.0 as usize];
                                attributed &= if real.hp > 0 {
                                    world.state.sides[1].active[0] == mon
                                } else {
                                    p.hp == 0
                                };
                                // Compare PP by move identity: two Protect stints, Sludge Bomb,
                                // Nasty Plot and Dark Pulse all belong to Zoroark, even after faint.
                                for m in real.move_slots().iter().filter(|m| m.pp < m.max_pp) {
                                    attributed &=
                                        p.move_slots().iter().any(|x| x.id == m.id && x.pp == m.pp);
                                }
                                let active = b.state.sides[1].active[0];
                                if active != MonId::NONE
                                    && b.state.pokemon[active.0 as usize].species
                                        == dex::SPECIES_BASTIODON
                                {
                                    let world_active = world.state.sides[1].active[0];
                                    attributed &= world_active != MonId::NONE
                                        && world.state.pokemon[world_active.0 as usize].species
                                            == dex::SPECIES_BASTIODON;
                                    let bastiodon = world.teams().sides[1]
                                        .sets
                                        .iter()
                                        .position(|s| s.species == dex::SPECIES_BASTIODON)
                                        .unwrap();
                                    let p = &world.state.pokemon[6 + bastiodon];
                                    for m in b.state.pokemon[active.0 as usize]
                                        .move_slots()
                                        .iter()
                                        .filter(|m| m.pp < m.max_pp)
                                    {
                                        attributed &= p
                                            .move_slots()
                                            .iter()
                                            .any(|x| x.id == m.id && x.pp == m.pp);
                                    }
                                }
                            }
                            if !attributed {
                                wrong_attribution += 1;
                                eprintln!(
                                    "decision {checked}, turn {}: a world misattributed Zoroark's active position, faint or PP",
                                    b.turn()
                                );
                            }
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!(
                            "decision {checked}, turn {}, {:?}: {e}; {}",
                            b.turn(),
                            b.side_request_kind(viewer),
                            w.last_failure()
                        );
                    }
                }
                // The spectator replay ends at this decision; it contains no turn-12 choices.
                if b.turn() == 12 && b.side_request_kind(viewer) == RequestKind::Move {
                    break 'battle;
                }
                w.push_choice(&choice_text(&b, side, &choice));
            }
            b.choose_typed(side, &choice).unwrap();
        }
        let lines = player_view(&b.log.entries, cursor, viewer);
        if std::env::var_os("NUMBRION_REPLAY_DUMP").is_some() {
            for line in &lines {
                eprintln!("{line}");
            }
        }
        w.push_lines(lines.iter().map(String::as_str));
        cursor = b.log.entries.len();
    }
    eprintln!(
        "real Zoroark: {checked} decisions, {failed} without worlds, {missing_illusion} missing pre-break Illusion, {wrong_attribution} wrong post-break attribution"
    );
    let lines = player_view(&b.log.entries, 0, viewer);
    let mut remaining = lines.as_slice();
    for prefix in [
        "|switch|p2a: Goodra|Goodra, L86, F|100/100",
        "|switch|p2b: Bastiodon|Bastiodon, L90, F|100/100",
        "|turn|1",
        "|move|p2b: Bastiodon|Protect|",
        "|turn|2",
        "|switch|p2b: Grimmsnarl|",
        "|turn|3",
        "|switch|p2a: Bastiodon|Bastiodon, L90, F|100/100",
        "|turn|4",
        "|move|p2a: Bastiodon|Protect|",
        "|turn|5",
        "|move|p2a: Bastiodon|Sludge Bomb|",
        "|-damage|p2a: Bastiodon|91/100|[from] item: Life Orb",
        "|turn|6",
        "|move|p2a: Bastiodon|Nasty Plot|",
        "|turn|7",
        "|-terastallize|p2a: Bastiodon|Poison",
        "|replace|p2a: Zoroark|Zoroark, L84, M",
        "|turn|8",
        "|move|p2a: Zoroark|Dark Pulse|",
        "|turn|9",
        "|faint|p2a: Zoroark",
        "|switch|p2a: Bastiodon|Bastiodon, L90, F|100/100",
        "|turn|10",
        "|move|p2a: Bastiodon|Iron Defense|",
        "|turn|11",
        "|move|p2a: Bastiodon|Foul Play|",
        "|switch|p1b: Bellossom|",
        "|turn|12",
    ] {
        let at = remaining
            .iter()
            .position(|s| s.starts_with(prefix))
            .unwrap_or_else(|| panic!("simulation lost replay milestone {prefix}"));
        remaining = &remaining[at + 1..];
    }
    assert_eq!(
        lines.iter().filter(|s| s.starts_with("|replace|")).count(),
        1,
        "lead must switch out unbroken"
    );
    assert_eq!(
        checked, 16,
        "twelve move decisions and four p1 replacement decisions"
    );
    assert_eq!(failed, 0, "worlds must succeed at every p1 boundary");
    assert_eq!(
        missing_illusion, 0,
        "evidence must produce an Illusion world before replace"
    );
    assert_eq!(
        wrong_attribution, 0,
        "every post-break world must attribute both disguised stints to Zoroark"
    );
}
