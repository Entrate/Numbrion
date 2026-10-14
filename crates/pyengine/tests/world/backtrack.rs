//! Collapse recovery by backtracking (`WorldBuilder::update`, `backtrack`). The GetErGoin fixture
//! (tera_reveal.rs): Lucario switches in on turn 4 and uses Flash Cannon on turn 5. The pool
//! makes a Lucario set without Flash Cannon far likelier, so every world draws it when Lucario
//! enters and every world dies at the reveal. Recovery must come from the worlds' snapshots taken
//! before Lucario entered, with its set drawn again under the reveal, not from rebuilds. Set repair
//! is turned off here; with it, the same reveal is repaired first.
use super::tera_reveal::{P1, P2, choose};
use super::*;
use engine::{dex, ids::EffectKind, search::world::WorldStats};

#[test]
fn a_mid_game_reveal_that_kills_every_world_is_recovered_by_backtracking() {
    let (before, after) = reveal(WorldConfig { repair_budget: 0, ..WorldConfig::default() });
    assert!(after.backtracks > before.backtracks, "no world was restored from a snapshot");
}

#[test]
fn set_repair_comes_before_backtracking() {
    let (before, after) = reveal(WorldConfig::default());
    assert!(after.set_repairs > before.set_repairs, "no world was repaired");
    assert_eq!(after.restores, before.restores, "repair should have made restores unnecessary");
}

/// Play the fixture to the Flash Cannon reveal; the builder's stats just before it and after it.
fn reveal(config: WorldConfig) -> (WorldStats, WorldStats) {
    let teams = [Arc::new(TeamDef::unpack(P1).unwrap()), Arc::new(TeamDef::unpack(P2).unwrap())];
    let mut sets = SetPool::new();
    for team in &teams {
        for set in &team.sets[..team.len as usize] {
            sets.add_set(set);
        }
    }
    let lucario = teams[1].sets[4].clone();
    let flash_cannon = dex::lookup(EffectKind::Move, "Flash Cannon").unwrap();
    let mut common = lucario.clone();
    let slot = common.moves.iter().position(|&m| m == flash_cannon).unwrap();
    common.moves[slot] = dex::lookup(EffectKind::Move, "Meteor Mash").unwrap();
    for _ in 0..300 {
        sets.add_set(&common);
    }
    let sets = Arc::new(sets);
    let mut b = B::from_team_defs([1, 7, 3, 1], teams[0].clone(), teams[1].clone(), names(), TextLog::default()).unwrap();
    b.start().unwrap();
    let viewer = SideId(0);
    let mut w = WorldBuilder::new(viewer, b.teams().sides[0].clone(), names(), sets, config, 42).unwrap();
    w.push_lines(player_view(&b.log.entries, 0, viewer).iter().map(String::as_str));
    let mut cursor = b.log.entries.len();
    let lucario_moves = |w: &WorldBuilder| -> Vec<bool> {
        w.opponent_teams()
            .iter()
            .filter_map(|t| t.sets[..t.len as usize].iter().find(|s| s.species == lucario.species))
            .map(|s| s.moves.contains(&flash_cannon))
            .collect()
    };
    let mut before = None;
    let mut revealed = false;
    while !b.state.ended && b.turn() <= 8 {
        for side in [viewer, SideId(1)] {
            if !needs(&b, side) {
                continue;
            }
            let choice = choose(&b, side);
            if side == viewer {
                w.push_request(&b.request_json(0).unwrap());
                if revealed {
                    let stats = w.stats;
                    w.worlds::<TextLog>(8, b.turn() as u64)
                        .unwrap_or_else(|e| panic!("turn {}: {e}; {}", b.turn(), w.last_failure()));
                    let before: WorldStats = before.expect("a decision before the reveal");
                    eprintln!("before the reveal {before:?}\nafter it {:?}", w.stats);
                    assert_eq!(w.stats.collapses, before.collapses + 1, "the reveal should kill every world");
                    assert_eq!(w.stats.builds - stats.builds, 1, "only the one diversity build of a refill");
                    assert!(lucario_moves(&w).iter().filter(|&&m| m).count() >= 7, "{:?}", lucario_moves(&w));
                    return (before, w.stats);
                }
                w.worlds::<TextLog>(8, b.turn() as u64)
                    .unwrap_or_else(|e| panic!("turn {}: {e}; {}", b.turn(), w.last_failure()));
                if lucario_moves(&w).len() == 8 {
                    assert!(lucario_moves(&w).iter().all(|&m| !m), "the pool should make every world's Lucario lack Flash Cannon");
                }
                before = Some(w.stats);
                w.push_choice(&choice_text(&b, side, &choice));
            }
            b.choose_typed(side, &choice).unwrap();
        }
        let lines = player_view(&b.log.entries, cursor, viewer);
        revealed |= lines.iter().any(|l| l.starts_with("|move|p2") && l.contains("|Flash Cannon|"));
        w.push_lines(lines.iter().map(String::as_str));
        cursor = b.log.entries.len();
    }
    panic!("Lucario never used Flash Cannon");
}
