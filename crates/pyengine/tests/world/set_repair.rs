//! Exact set repair (`repair_sets`): a revealed move that every world's set lacks is fixed by giving
//! the dead worlds another pool set before the Pokemon entered (here a lead: the snapshot before the
//! start) and replaying their recorded segments, without rebuilding or backtracking.
use super::*;
use engine::dex;

#[test]
fn a_revealed_move_repairs_worlds_without_rebuilding() {
    repair_reveal(false);
}

#[test]
fn a_revealed_move_is_repaired_after_an_off_pool_tera_reveal() {
    repair_reveal(true);
}

fn repair_reveal(off_pool_tera: bool) {
    let own = Arc::new(TeamDef::unpack("Latias||SoulDew|Levitate|mistball,protect,dracometeor,tailwind||85,,85,85,85,85|F|,0,,,,||80|,,,,,Steel]\
Umbreon||Leftovers|InnerFocus|snarl,protect,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Dark").unwrap());
    let opp = Arc::new(TeamDef::unpack("Lucario||LifeOrb|InnerFocus|closecombat,extremespeed,flashcannon,protect||85,85,85,85,85,85|M|||86|,,,,,Normal]\
Mesprit||LifeOrb|Levitate|protect,psychic,nastyplot,dazzlinggleam||85,,85,85,85,85|N|,0,,,,||86|,,,,,Psychic").unwrap());
    let mut sets = SetPool::new();
    for s in &opp.sets[..2] {
        let mut set = s.clone();
        if off_pool_tera && set.species == dex::SPECIES_LUCARIO {
            set.tera_type = dex::type_id("Dragon").unwrap();
        }
        sets.add_set(&set);
    }
    let mut wrong = opp.sets[0].clone();
    if off_pool_tera { wrong.tera_type = dex::type_id("Dragon").unwrap(); }
    wrong.moves[2] = dex::MOVE_AURASPHERE;
    for _ in 0..300 { sets.add_set(&wrong); }
    let mut b = B::from_team_defs([1, 7, 3, 1], own, opp, names(), TextLog::default()).unwrap();
    b.start().unwrap();
    let config = WorldConfig { particles: 8, rebuild_budget: 1, fresh_per_update: 0, ..WorldConfig::default() };
    let mut w = WorldBuilder::new(SideId(0), b.teams().sides[0].clone(), names(), Arc::new(sets), config, 42).unwrap();
    w.push_lines(player_view(&b.log.entries, 0, SideId(0)).iter().map(String::as_str));
    w.push_request(&b.request_json(0).unwrap());
    w.update().unwrap();
    assert!(w.opponent_teams().iter().all(|t| !t.sets[0].moves.contains(&dex::MOVE_FLASHCANNON)));
    let builds = w.stats.builds;
    let mut cursor = b.log.entries.len();
    for turn in 1..=3 {
        w.push_choice("move 2, move 2");
        b.choose(0, "move 2, move 2").unwrap();
        b.choose(1, if turn == 1 {
            if off_pool_tera { "move 4 terastallize, move 1" } else { "move 4, move 1" }
        } else { "move 3 +1, move 1" }).unwrap();
        w.push_lines(player_view(&b.log.entries, cursor, SideId(0)).iter().map(String::as_str));
        cursor = b.log.entries.len();
        w.push_request(&b.request_json(0).unwrap());
        let worlds = w.worlds::<TextLog>(8, turn).unwrap_or_else(|e| panic!("turn {turn}: {e}; {}", w.last_failure()));
        for world in worlds { assert_eq!(world.request_json(0), b.request_json(0)); }
        assert_eq!(w.stats.builds, builds, "turn {turn}: repair must keep the original worlds; {:?}; {}", w.stats, w.last_failure());
        assert_eq!(w.stats.deaths, if turn == 1 { 0 } else { 8 }, "turn {turn}: only the reveal kills worlds");
    }
    // One world comes back; copies of it fill the rest.
    assert_eq!((w.stats.collapses, w.stats.set_repairs, w.stats.restores), (1, 1, 0));
    assert!(w.stats.repair_replays >= 2, "the start and the earlier turn must both be checked");
    assert!(w.opponent_teams().iter().all(|t| t.sets[0].moves.contains(&dex::MOVE_FLASHCANNON)));
    if off_pool_tera {
        assert!(w.opponent_teams().iter().all(|t| t.sets[0].tera_type == dex::type_id("Normal").unwrap()));
    }
}

#[test]
fn repair_can_be_turned_off() {
    let own = Arc::new(TeamDef::unpack("Latias||SoulDew|Levitate|mistball,protect,dracometeor,tailwind||85,,85,85,85,85|F|,0,,,,||80|,,,,,Steel]\
Umbreon||Leftovers|InnerFocus|snarl,protect,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Dark").unwrap());
    let opp = Arc::new(TeamDef::unpack("Lucario||LifeOrb|InnerFocus|closecombat,extremespeed,flashcannon,protect||85,85,85,85,85,85|M|||86|,,,,,Normal]\
Mesprit||LifeOrb|Levitate|protect,psychic,nastyplot,dazzlinggleam||85,,85,85,85,85|N|,0,,,,||86|,,,,,Psychic").unwrap());
    let mut sets = SetPool::new();
    for s in &opp.sets[..2] { sets.add_set(s); }
    let mut wrong = opp.sets[0].clone();
    wrong.moves[2] = dex::MOVE_AURASPHERE;
    for _ in 0..300 { sets.add_set(&wrong); }
    let mut b = B::from_team_defs([1, 7, 3, 1], own, opp, names(), TextLog::default()).unwrap();
    b.start().unwrap();
    let config = WorldConfig { particles: 8, rebuild_budget: 1, fresh_per_update: 0, repair_budget: 0, ..WorldConfig::default() };
    let mut w = WorldBuilder::new(SideId(0), b.teams().sides[0].clone(), names(), Arc::new(sets), config, 42).unwrap();
    w.push_lines(player_view(&b.log.entries, 0, SideId(0)).iter().map(String::as_str));
    w.push_request(&b.request_json(0).unwrap());
    w.update().unwrap();
    let mut cursor = b.log.entries.len();
    for turn in 1..=2 {
        w.push_choice("move 2, move 2");
        b.choose(0, "move 2, move 2").unwrap();
        b.choose(1, if turn == 1 { "move 4, move 1" } else { "move 3 +1, move 1" }).unwrap();
        w.push_lines(player_view(&b.log.entries, cursor, SideId(0)).iter().map(String::as_str));
        cursor = b.log.entries.len();
        w.push_request(&b.request_json(0).unwrap());
        let _ = w.update();
    }
    assert_eq!(w.stats.collapses, 1);
    assert_eq!((w.stats.set_repair_attempts, w.stats.set_repairs, w.stats.repair_replays), (0, 0, 0));
}

/// Zamazenta's Crowned forme is set up at the battle start, so `introduce` cannot swap it into a
/// hidden slot later and every world dies when it enters. Repair gives it a slot that was hidden
/// from the start instead, without rebuilding.
#[test]
fn a_battle_start_species_entering_later_is_repaired_from_the_start() {
    const Z: &str = "Zamazenta||RustedShield|DauntlessShield|heavyslam,bodypress,wideguard,irondefense||85,85,85,85,85,85|N|||67|,,,,,Fire";
    let own = Arc::new(TeamDef::unpack("Latias||SoulDew|Levitate|mistball,protect,dracometeor,tailwind||85,,85,85,85,85|F|,0,,,,||80|,,,,,Steel]\
Blastoise||WhiteHerb|Torrent|shellsmash,protect,muddywater,dragonpulse||85,,85,85,85,85|F|,0,,,,||83|,,,,,Dragon").unwrap());
    let opp = Arc::new(TeamDef::unpack(&format!("Mesprit||LifeOrb|Levitate|protect,psychic,nastyplot,dazzlinggleam||85,,85,85,85,85|N|,0,,,,||86|,,,,,Psychic]\
Umbreon||Leftovers|InnerFocus|snarl,protect,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Dark]{Z}")).unwrap());
    let mut sets = SetPool::new();
    for s in &opp.sets[..3] { sets.add_set(s); }
    let filler = TeamDef::unpack("Lucario||LifeOrb|InnerFocus|closecombat,extremespeed,flashcannon,protect||85,85,85,85,85,85|M|||86|,,,,,Normal").unwrap();
    for _ in 0..300 { sets.add_set(&filler.sets[0]); }
    let mut b = B::from_team_defs([1, 7, 3, 1], own, opp, names(), TextLog::default()).unwrap();
    b.start().unwrap();
    let config = WorldConfig { particles: 8, rebuild_budget: 1, fresh_per_update: 0, ..WorldConfig::default() };
    let mut w = WorldBuilder::new(SideId(0), b.teams().sides[0].clone(), names(), Arc::new(sets), config, 42).unwrap();
    w.push_lines(player_view(&b.log.entries, 0, SideId(0)).iter().map(String::as_str));
    w.push_request(&b.request_json(0).unwrap());
    w.update().unwrap();
    let builds = w.stats.builds;
    assert!(w.opponent_teams().iter().all(|t| t.sets[..t.len as usize].iter().all(|s| s.name != "Zamazenta")));
    let cursor = b.log.entries.len();
    w.push_choice("move 2, move 2");
    b.choose(0, "move 2, move 2").unwrap();
    b.choose(1, "switch 3, move 2").unwrap();
    w.push_lines(player_view(&b.log.entries, cursor, SideId(0)).iter().map(String::as_str));
    w.push_request(&b.request_json(0).unwrap());
    let worlds = w.worlds::<TextLog>(8, 1).unwrap_or_else(|e| panic!("{e}; {}", w.last_failure()));
    for world in worlds { assert_eq!(world.request_json(0), b.request_json(0)); }
    assert_eq!(w.stats.builds, builds, "{:?}; {}", w.stats, w.last_failure());
    assert_eq!((w.stats.collapses, w.stats.set_repairs), (1, 1));
    assert!(w.opponent_teams().iter().all(|t| t.sets[..t.len as usize].iter().any(|s| s.name == "Zamazenta")));
}
