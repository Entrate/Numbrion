//! Engine-generated regressions from the Illusion correctness review.
use engine::{
    Battle, dex,
    ids::*,
    log::TextLog,
    search::{
        sets::SetPool,
        world::{WorldBuilder, WorldConfig},
    },
    teams::{SetDef, TeamDef},
};
use std::sync::Arc;
fn set(s: &str) -> SetDef {
    TeamDef::unpack(s).unwrap().sets[0].clone()
}
fn team(ss: Vec<SetDef>) -> Arc<TeamDef> {
    let mut t = TeamDef::default();
    t.len = ss.len() as u8;
    for (i, s) in ss.into_iter().enumerate() {
        t.sets[i] = s;
    }
    Arc::new(t)
}
fn view(entries: &[String], from: usize) -> Vec<String> {
    let mut out = vec![];
    let mut i = from;
    while i < entries.len() {
        if entries[i].starts_with("|split|") {
            out.push(entries[i + if entries[i] == "|split|p1" { 1 } else { 2 }].clone());
            i += 3;
        } else {
            out.push(entries[i].clone());
            i += 1;
        }
    }
    out
}
#[test]
fn older_illusion_lead_remains_reachable() {
    let z = set(
        "Zoroark|ZoroarkHisui|LifeOrb|Illusion|protect,nastyplot,sludgebomb,darkpulse||85,,85,85,85,85|F|,0,,,,||84|,,,,,Poison",
    );
    let b = set(
        "Darkrai||Leftovers|BadDreams|protect,nastyplot,darkpulse,sludgebomb||85,,85,85,85,85|N|,0,,,,||80|,,,,,Fighting",
    );
    let u = set(
        "Umbreon||Leftovers|InnerFocus|protect,snarl,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Dark",
    );
    let l = set(
        "Latias||SoulDew|Levitate|protect,recover,dracometeor,tailwind||85,,85,85,85,85|F|,0,,,,||80|,,,,,Steel",
    );
    let m = set(
        "Mesprit||LifeOrb|Levitate|protect,psychic,nastyplot,dazzlinggleam||85,,85,85,85,85|N|,0,,,,||86|,,,,,Psychic",
    );
    let bl = set(
        "Blastoise||WhiteHerb|Torrent|protect,shellsmash,muddywater,dragonpulse||85,,85,85,85,85|F|,0,,,,||83|,,,,,Dragon",
    );
    let opp = team(vec![
        z.clone(),
        m.clone(),
        u.clone(),
        l.clone(),
        bl.clone(),
        b.clone(),
    ]);
    let mut own_l = l.clone();
    own_l.moves[0] = dex::MOVE_AURASPHERE;
    let own = team(vec![
        own_l,
        u.clone(),
        bl.clone(),
        m.clone(),
        b.clone(),
        z.clone(),
    ]);
    let mut pool = SetPool::new();
    for s in &opp.sets {
        pool.add_set(s);
    }
    let pool = Arc::new(pool);
    for swaps in [2usize, 8] {
        let mut real = Battle::from_team_defs(
            [1, 2, 3, 4],
            own.clone(),
            opp.clone(),
            ["A".into(), "B".into()],
            TextLog::default(),
        )
        .unwrap();
        real.start().unwrap();
        let mut w = WorldBuilder::new(
            SideId(0),
            own.clone(),
            ["A".into(), "B".into()],
            pool.clone(),
            WorldConfig {
                particles: 1,
                rebuild_budget: 16,
                segment_budget: 300,
                ..WorldConfig::default()
            },
            41,
        )
        .unwrap();
        let mut cursor = 0;
        let lines = view(&real.log.entries, cursor);
        w.push_lines(lines.iter().map(String::as_str));
        cursor = real.log.entries.len();
        w.push_choice("move 1 +1, move 1");
        real.choose(0, "move 1 +1, move 1").unwrap();
        real.choose(1, "move 2, move 1").unwrap();
        let lines = view(&real.log.entries, cursor);
        w.push_lines(lines.iter().map(String::as_str));
        cursor = real.log.entries.len();
        for turn in 0..=swaps {
            let ownch = "move 4, move 1";
            let target = if turn == 0 {
                dex::SPECIES_UMBREON
            } else if turn == swaps {
                dex::SPECIES_ZOROARKHISUI
            } else if turn % 2 == 1 {
                dex::SPECIES_LATIAS
            } else {
                dex::SPECIES_UMBREON
            };
            let party = real.state.sides[1].party;
            let slot = party
                .iter()
                .position(|id| id.0 < 12 && real.state.pokemon[id.0 as usize].species == target)
                .unwrap();
            let oppch = format!("switch {}, move 1", slot + 1);
            w.push_choice(ownch);
            real.choose(0, ownch).unwrap();
            real.choose(1, &oppch).unwrap();
            let lines = view(&real.log.entries, cursor);
            w.push_lines(lines.iter().map(String::as_str));
            cursor = real.log.entries.len();
        }
        // Hit the returning disguised Zoroark with Latias's Draco Meteor to get replace.
        let ownch = "move 3 +1, move 1";
        w.push_choice(ownch);
        real.choose(0, ownch).unwrap();
        real.choose(1, "move 2, move 1").unwrap();
        let lines = view(&real.log.entries, cursor);
        w.push_lines(lines.iter().map(String::as_str));

        w.push_request(&real.request_json(0).unwrap());
        let r = w.update();
        assert!(r.is_ok(), "{r:?}: {}", w.last_failure());
        assert_eq!(w.alive(), 1);
        let worlds = w.worlds::<TextLog>(1, 7).unwrap();
        assert_eq!(worlds[0].request_json(0), real.request_json(0));
    }
}
#[test]
fn repaired_illusion_proposal_is_retained() {
    let z = set(
        "Zoroark|ZoroarkHisui|LifeOrb|Illusion|nastyplot,protect,darkpulse,sludgebomb||85,,85,85,85,85|F|,0,,,,||84|,,,,,Poison",
    );
    let b = set(
        "Darkrai||Leftovers|BadDreams|nastyplot,protect,darkpulse,sludgebomb||85,,85,85,85,85|N|,0,,,,||80|,,,,,Fighting",
    );
    let l = set(
        "Latias||SoulDew|Levitate|toxic,protect,dracometeor,aurasphere||85,,85,85,85,85|F|,0,,,,||80|,,,,,Steel",
    );
    let u = set(
        "Umbreon||Leftovers|InnerFocus|protect,snarl,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Dark",
    );
    let mut actual_z = z.clone();
    actual_z.item = dex::ITEM_LEFTOVERS;
    let mut actual_b = b.clone();
    actual_b.item = EffectId::NONE;
    let opp = team(vec![actual_z, actual_b]);
    let own = team(vec![l, u]);
    for include_actual in [false, true] {
        let mut pool = SetPool::new();
        pool.add_set(&z);
        pool.add_set(&b);
        if include_actual {
            let mut actual = z.clone();
            actual.item = dex::ITEM_LEFTOVERS;
            pool.add_set(&actual);
        }
        let pool = Arc::new(pool);
        let mut real = Battle::from_team_defs(
            [1, 2, 3, 4],
            own.clone(),
            opp.clone(),
            ["A".into(), "B".into()],
            TextLog::default(),
        )
        .unwrap();
        real.start().unwrap();
        let mut w = WorldBuilder::new(
            SideId(0),
            own.clone(),
            ["A".into(), "B".into()],
            pool,
            WorldConfig {
                particles: 1,
                rebuild_budget: 16,
                segment_budget: 300,
                ..WorldConfig::default()
            },
            41,
        )
        .unwrap();
        let lines = view(&real.log.entries, 0);
        w.push_lines(lines.iter().map(String::as_str));
        let mut cursor = real.log.entries.len();
        w.push_choice("move 1 +1, move 1");
        real.choose(0, "move 1 +1, move 1").unwrap();
        real.choose(1, "move 1, move 2").unwrap();
        let lines = view(&real.log.entries, cursor);
        w.push_lines(lines.iter().map(String::as_str));
        cursor = real.log.entries.len();
        w.push_choice("move 4 +1, move 1");
        real.choose(0, "move 4 +1, move 1").unwrap();
        real.choose(1, "move 1, move 2").unwrap();
        let lines = view(&real.log.entries, cursor);
        w.push_lines(lines.iter().map(String::as_str));
        w.push_request(&real.request_json(0).unwrap());
        let r = w.update();
        assert!(r.is_ok(), "{r:?}: {}", w.last_failure());
        assert_eq!(w.alive(), 1);
        let worlds = w.worlds::<TextLog>(1, 7).unwrap();
        assert_eq!(worlds[0].request_json(0), real.request_json(0));
    }
}
