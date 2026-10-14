//! Regression from ladder replay Gen9RandomDoublesBattle-2026-10-13-numbrion-getergoin (Numbrion =
//! p1, sets from pool-s1-200k.txt.gz): search failed from turn 12 on, after Eternatus terastallized
//! to Water on turn 11. Copies of one survivor had all drawn Tera Dragon, so the reveal killed every
//! world and a rebuild from turn 1 ran out of budget. Here Eternatus terastallizes on turn 2 and the
//! pool makes Tera Dragon far likelier, so every world holds the wrong type until the reveal.
use super::*;
use engine::{dex, ids::EffectKind};

pub(super) const P1: &str = "Lurantis||AssaultVest|Contrary|leafblade,leafstorm,pollenpuff,superpower||85,85,85,85,85,85|M|||85|,,,,,Fighting]\
Magmar||Eviolite|FlameBody|followme,heatwave,knockoff,willowisp||85,85,85,85,85,85|M|||84|,,,,,Grass]\
Tsareena||AssaultVest|QueenlyMajesty|highjumpkick,powerwhip,rapidspin,tripleaxel||85,85,85,85,85,85|F|||85|,,,,,Fighting]\
Kingambit||SitrusBerry|Defiant|ironhead,kowtowcleave,protect,suckerpunch||85,85,85,85,85,85|F|||77|,,,,,Flying]\
Crawdaunt||ChoiceBand|Adaptability|aquajet,closecombat,crabhammer,knockoff||85,85,85,85,85,85|M|||87|,,,,,Fighting]\
Cacturne||LifeOrb|WaterAbsorb|leafstorm,knockoff,suckerpunch,spikyshield||85,85,85,85,85,85|F|||92|,,,,,Poison";
pub(super) const P2: &str = "Chi-Yu||LifeOrb|BeadsofRuin|darkpulse,heatwave,nastyplot,protect||85,,85,85,85,85|N|,0,,,,||75|,,,,,Water]\
Eternatus||Leftovers|Pressure|cosmicpower,dynamaxcannon,flamethrower,recover||85,,85,85,85,85|N|,0,,,,||70|,,,,,Water]\
Politoed||ChoiceSpecs|Drizzle|hydropump,icebeam,muddywater,weatherball||85,,85,85,85,85|F|,0,,,,||82|,,,,,Water]\
Spectrier||LifeOrb|GrimNeigh|drainingkiss,nastyplot,protect,shadowball||85,,85,85,85,85|N|,0,,,,||77|,,,,,Fairy]\
Lucario||LifeOrb|InnerFocus|closecombat,extremespeed,flashcannon,protect||85,85,85,85,85,85|M|||86|,,,,,Normal]\
Roaring Moon||BoosterEnergy|Protosynthesis|acrobatics,breakingswipe,knockoff,protect||85,85,85,85,85,85|N|||76|,,,,,Flying";

/// The replay's choice for one slot: `Move target [tera]` or `switch Species`.
fn plan(side: u8, turn: u32) -> [&'static str; 2] {
    match (side, turn) {
        (0, 1) => ["Superpower 1", "Knock Off 2"],
        (0, 2) => ["Superpower 1", "switch Tsareena"],
        (0, 3) => ["switch Kingambit", "Triple Axel 2"],
        (0, 4) => ["Kowtow Cleave 1", "Rapid Spin 2"],
        (0, 5) => ["Protect 0", "Triple Axel 2"],
        (0, 6) => ["Sucker Punch 2", "High Jump Kick 1"],
        (0, 7) => ["Protect 0", "Power Whip 1"],
        (0, 8) => ["Kowtow Cleave 1", "Triple Axel 2"],
        (0, 9) => ["Kowtow Cleave 2", "Triple Axel 2"],
        (0, 10) => ["Kowtow Cleave 2", "Close Combat 1"],
        (0, 11) => ["Kowtow Cleave 2", "switch Magmar"],
        (0, 12) => ["Kowtow Cleave 2", "Will-O-Wisp 2"],
        (0, 13..=15) => ["Kowtow Cleave 2", "Follow Me 0"],
        (0, 16) => ["switch Lurantis", "Follow Me 0"],
        (0, _) => ["Leaf Storm 2", "Knock Off 2"],
        (1, 1) => ["Nasty Plot 0", "Cosmic Power 0"],
        (1, 2) => ["switch Spectrier", "Dynamax Cannon 1 tera"],
        (1, 3) => ["Protect 0", "Dynamax Cannon 1"],
        (1, 4) => ["switch Lucario", "Recover 0"],
        (1, 5) => ["Flash Cannon 2", "Flamethrower 2"],
        (1, 6) => ["Flash Cannon 2", "Recover 0"],
        (1, 7) => ["Ice Beam 1", "Flamethrower 1"],
        (1, 8) => ["Nasty Plot 0", "Cosmic Power 0"],
        (1, 9) => ["Acrobatics 2", "Recover 0"],
        (1, 10) => ["Knock Off 2", "Recover 0"],
        (1, 12) | (1, 14) | (1, 18) => ["Protect 0", "Recover 0"],
        (1, 13) => ["Protect 0", "Flamethrower 1"],
        (1, _) => ["Protect 0", "Dynamax Cannon 2"],
        _ => unreachable!(),
    }
}

fn replacement(side: u8, turn: u32) -> &'static str {
    match (side, turn) {
        (0, _) => "Crawdaunt",
        (1, 0..=1) => "Politoed",
        (1, 2..=6) => "Politoed",
        (1, 7) => "Spectrier",
        _ => "Roaring Moon",
    }
}

pub(super) fn choose(b: &B, side: SideId) -> Vec<SlotChoice> {
    let turn = b.turn();
    let forced = b.side_request_kind(side) == RequestKind::Switch;
    let want = plan(side.0, turn);
    let legal = b.legal_actions(side);
    let party = b.state.sides[side.0 as usize].party;
    let mut out: Vec<SlotChoice> = Vec::new();
    for (slot, ls) in legal.slots[..legal.slot_count as usize].iter().enumerate() {
        if ls.automatic_pass {
            out.push(SlotChoice::default());
            continue;
        }
        let w = if forced { format!("switch {}", replacement(side.0, turn)) } else { want[slot].to_string() };
        if let Some(name) = w.strip_prefix("switch ").or(forced.then_some("")) {
            let free = |p: &usize| {
                ls.switch_mask & (1 << p) != 0
                    && !out.iter().any(|c| c.kind == ChoiceKind::Switch && c.switch_to == party[*p])
            };
            let species = dex::lookup(EffectKind::Species, name);
            let pick = (0..6)
                .filter(free)
                .find(|&p| Some(b.state.pokemon[party[p].0 as usize].species) == species)
                .or_else(|| (0..6).find(free));
            if let Some(p) = pick {
                out.push(SlotChoice { kind: ChoiceKind::Switch, switch_to: party[p], ..SlotChoice::default() });
                continue;
            }
            if forced {
                out.push(SlotChoice::default());
                continue;
            }
        }
        let parts: Vec<&str> = w.rsplitn(3, ' ').collect();
        let (tera, target, name) = if parts[0] == "tera" {
            let rest: Vec<&str> = w.trim_end_matches(" tera").rsplitn(2, ' ').collect();
            (true, rest[0].parse::<i8>().unwrap_or(0), rest[1].to_string())
        } else {
            let rest: Vec<&str> = w.rsplitn(2, ' ').collect();
            (false, rest[0].parse::<i8>().unwrap_or(0), rest.get(1).copied().unwrap_or("").to_string())
        };
        let moves = &ls.moves[..ls.move_count as usize];
        let wanted = dex::lookup(EffectKind::Move, &name);
        let m = moves.iter().find(|m| Some(m.move_id) == wanted).unwrap_or(&moves[0]);
        let loc = if m.targets.contains(target) {
            target
        } else {
            [0, 1, 2, -1, -2].into_iter().find(|&l| m.targets.contains(l)).unwrap()
        };
        out.push(SlotChoice {
            kind: ChoiceKind::Move,
            move_slot: m.move_slot,
            move_id: m.move_id,
            move_kind: m.move_kind,
            target_loc: loc,
            tera: tera && m.can_terastallize,
            ..SlotChoice::default()
        });
    }
    if !b.is_legal_joint_choice(side, &out) {
        for c in &mut out {
            c.tera = false;
        }
    }
    assert!(b.is_legal_joint_choice(side, &out), "turn {turn} p{}: {out:?}", side.0 + 1);
    out
}

#[test]
fn a_tera_reveal_retypes_worlds_instead_of_killing_them() {
    let teams = [Arc::new(TeamDef::unpack(P1).unwrap()), Arc::new(TeamDef::unpack(P2).unwrap())];
    let mut sets = SetPool::new();
    for team in &teams {
        for set in &team.sets[..team.len as usize] {
            sets.add_set(set);
        }
    }
    let eternatus = teams[1].sets[1].clone();
    let dragon = SetDef { tera_type: engine::dex::type_id("Dragon").unwrap(), ..eternatus.clone() };
    for _ in 0..300 {
        sets.add_set(&dragon);
    }
    let sets = Arc::new(sets);
    let mut b = B::from_team_defs([1, 7, 3, 1], teams[0].clone(), teams[1].clone(), names(), TextLog::default()).unwrap();
    b.start().unwrap();
    let viewer = SideId(0);
    let mut w = WorldBuilder::new(viewer, b.teams().sides[0].clone(), names(), sets, WorldConfig::default(), 42).unwrap();
    w.push_lines(player_view(&b.log.entries, 0, viewer).iter().map(String::as_str));
    let mut cursor = b.log.entries.len();
    let eternatus_tera = |w: &WorldBuilder| -> Vec<_> {
        w.opponent_teams().iter().map(|t| t.sets[..t.len as usize].iter().find(|s| s.species == dex::SPECIES_ETERNATUS).unwrap().tera_type).collect()
    };
    let mut before = None;
    while b.turn() <= 3 {
        for side in [viewer, SideId(1)] {
            if !needs(&b, side) {
                continue;
            }
            let choice = choose(&b, side);
            if side == viewer {
                w.push_request(&b.request_json(0).unwrap());
                w.worlds::<TextLog>(8, b.turn() as u64).unwrap_or_else(|e| panic!("turn {}: {e}; {}", b.turn(), w.last_failure()));
                if b.turn() == 2 {
                    assert!(eternatus_tera(&w).iter().all(|&t| t == dragon.tera_type), "the pool should make every world draw Tera Dragon");
                    before = Some(w.stats);
                } else if b.turn() == 3 {
                    let before = before.expect("a turn-2 decision");
                    assert_eq!((w.stats.deaths, w.stats.builds), (before.deaths, before.builds), "the reveal killed worlds: {}", w.last_failure());
                    assert!(eternatus_tera(&w).iter().all(|&t| t == eternatus.tera_type));
                    return;
                }
                w.push_choice(&choice_text(&b, side, &choice));
            }
            b.choose_typed(side, &choice).unwrap();
        }
        let lines = player_view(&b.log.entries, cursor, viewer);
        w.push_lines(lines.iter().map(String::as_str));
        cursor = b.log.entries.len();
    }
    panic!("the battle never reached a turn-3 decision");
}
