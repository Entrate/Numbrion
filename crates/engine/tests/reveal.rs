//! `RevealTracker` soundness (search/reveal.rs): random-policy battles of constructor.tsv teams,
//! each side's player view resolved like `LogViews::absorb` (crates/pyengine/src/game.rs) and fed
//! to a tracker per viewer; everything the tracker reports about either side must agree with the
//! true teams (species / level, moves ⊆ true moves, starting item, starting ability, Tera type).
//!
//! Sides carrying an Illusion user only check identity for their other Pokemon: a disguise that
//! breaks by fainting is invisible in the protocol, so its observations stay on the imitated
//! Pokemon. The Illusion user's own record (built from `|replace|`) is checked in full.
//!
//! `NUMBRION_TEST_BATTLES` scales the battle count (default 2000).
#![cfg(feature = "search")]
use engine::{
    Battle, dex,
    ids::{EffectId, SideId},
    log::TextLog,
    search::reveal::RevealTracker,
    state::choices::{ChoiceKind, RequestKind, SlotChoice},
    teams::TeamDef,
};
use std::sync::Arc;

type B = Battle<TextLog>;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < p
    }
    fn seed(&mut self) -> [u16; 4] {
        let x = self.next();
        [
            x as u16,
            (x >> 16) as u16,
            (x >> 32) as u16,
            (x >> 48) as u16,
        ]
    }
}

fn needs_action(b: &B, side: SideId) -> bool {
    matches!(
        b.side_request_kind(side),
        RequestKind::Move | RequestKind::Switch
    ) && b.legal_actions(side).slot_count > 0
}

fn pick(b: &B, side: SideId, rng: &mut Rng) -> Vec<SlotChoice> {
    let legal = b.legal_actions(side);
    let party = b.state.sides[side.0 as usize].party;
    let options: Vec<(Vec<SlotChoice>, Vec<SlotChoice>)> = legal.slots[..legal.slot_count as usize]
        .iter()
        .map(|ls| {
            let (mut moves, mut other) = (vec![], vec![]);
            if ls.automatic_pass {
                other.push(SlotChoice::default());
                return (moves, other);
            }
            for m in &ls.moves[..ls.move_count as usize] {
                for loc in -2..=2 {
                    for tera in [false, true] {
                        if m.targets.contains(loc) && (!tera || m.can_terastallize) {
                            moves.push(SlotChoice {
                                kind: ChoiceKind::Move,
                                move_slot: m.move_slot,
                                target_loc: loc,
                                tera,
                                move_id: m.move_id,
                                move_kind: m.move_kind,
                                ..SlotChoice::default()
                            });
                        }
                    }
                }
            }
            for p in 0..6 {
                if (ls.switch_mask | ls.revival_mask) & (1 << p) != 0 {
                    other.push(SlotChoice {
                        kind: ChoiceKind::Switch,
                        switch_to: party[p],
                        ..SlotChoice::default()
                    });
                }
            }
            if ls.can_pass {
                other.push(SlotChoice::default());
            }
            (moves, other)
        })
        .collect();
    for _ in 0..200 {
        let choice: Vec<_> = options
            .iter()
            .map(|(moves, other)| {
                let list = if moves.is_empty() || (!other.is_empty() && rng.chance(0.15)) {
                    other
                } else {
                    moves
                };
                list[rng.below(list.len())]
            })
            .collect();
        if b.is_legal_joint_choice(side, &choice) {
            return choice;
        }
    }
    panic!("no legal joint choice found for side {}", side.0);
}

fn load() -> Vec<([u16; 4], TeamDef, TeamDef)> {
    let mut battles = vec![];
    for line in include_str!("data/constructor.tsv")
        .lines()
        .filter(|s| s.starts_with("B\t"))
    {
        let f: Vec<_> = line.split('\t').collect();
        let seed: Vec<u16> = f[1].split(',').map(|n| n.parse().unwrap()).collect();
        let p1 = TeamDef::unpack(f[3]).unwrap();
        let p2 = TeamDef::unpack(f[4]).unwrap();
        if p1.len < 2 || p2.len < 2 {
            continue;
        }
        battles.push((seed.try_into().unwrap(), p1, p2));
    }
    battles
}

/// `extractChannelMessages`: the owner of a `|split|pN` triple sees the secret line, the other
/// side the shared one; empty lines are dropped.
fn player_views(entries: &[String]) -> [Vec<&str>; 2] {
    let mut out = [vec![], vec![]];
    let mut i = 0;
    while i < entries.len() {
        let line = entries[i].as_str();
        if let Some(n) = line.strip_prefix("|split|p")
            && let Ok(owner) = n.parse::<usize>()
            && (1..=2).contains(&owner)
            && i + 2 < entries.len()
        {
            for (p, view) in out.iter_mut().enumerate() {
                let l = if p + 1 == owner {
                    &entries[i + 1]
                } else {
                    &entries[i + 2]
                };
                if !l.is_empty() {
                    view.push(l.as_str());
                }
            }
            i += 3;
            continue;
        }
        if !line.is_empty() {
            for view in &mut out {
                view.push(line);
            }
        }
        i += 1;
    }
    out
}

fn play(seed: [u16; 4], p1: TeamDef, p2: TeamDef, rng: &mut Rng) -> B {
    let mut b = B::from_team_defs(
        seed,
        Arc::new(p1),
        Arc::new(p2),
        ["Player 1".into(), "Player 2".into()],
        TextLog::default(),
    )
    .unwrap();
    b.start().unwrap();
    let mut steps = 0;
    while !b.state.ended && steps < 400 {
        for side in [SideId(0), SideId(1)] {
            if needs_action(&b, side) {
                let choice = pick(&b, side, rng);
                b.choose_typed(side, &choice).unwrap();
            }
        }
        steps += 1;
    }
    b
}

#[derive(Default, Debug)]
struct Counts {
    battles: usize,
    mons: usize,
    moves: usize,
    starting_items: usize,
    starting_abilities: usize,
    tera_types: usize,
    illusion_sides: usize,
    replaced: usize,
}

fn name(id: EffectId) -> &'static str {
    if id == EffectId::NONE {
        "(none)"
    } else {
        dex::effect(id).name
    }
}

/// The details print the stored forme, which may be a battle-start forme of the set's species
/// (a Zacian set holding Rusted Sword shows as Zacian-Crowned).
fn species_ok(shown: EffectId, truth: EffectId) -> bool {
    if shown == EffectId::NONE {
        return false;
    }
    let s = dex::species(shown);
    shown == truth || s.battle_only.contains(&truth) || s.changes_from == truth
}

#[test]
fn revealed_information_is_sound() {
    let n: usize = std::env::var("NUMBRION_TEST_BATTLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let battles = load();
    let mut rng = Rng(0x2E7E_A1ED);
    let mut c = Counts::default();
    let mut failures = vec![];
    for i in 0..n {
        let (seed, p1, p2) = battles[i % battles.len()].clone();
        let seed = if i < battles.len() { seed } else { rng.seed() };
        let b = play(seed, p1, p2, &mut rng);
        c.battles += 1;
        let views = player_views(&b.log.entries);
        for (viewer, view) in views.iter().enumerate() {
            let mut t = RevealTracker::new();
            t.feed_all(view.iter().copied());
            assert_eq!(t.turn(), b.turn(), "battle {i}");
            for s in 0..2u8 {
                let side = SideId(s);
                let team = &b.teams().sides[s as usize];
                let sets = &team.sets[..team.len as usize];
                let illusion = sets.iter().any(|x| x.ability == dex::ABILITY_ILLUSION);
                c.illusion_sides += illusion as usize;
                assert_eq!(t.team_size(side), Some(team.len), "battle {i}");
                assert!(
                    t.leads(side).iter().all(Option::is_some),
                    "battle {i}: leads {:?}",
                    t.leads(side)
                );
                assert!(
                    t.mons(side).len() <= team.len as usize,
                    "battle {i}: more records than Pokemon"
                );
                for m in t.mons(side) {
                    c.mons += 1;
                    c.replaced += m.illusion as usize;
                    let mut fail = |what: String| {
                        failures.push(format!(
                            "battle {i} viewer {viewer} side {s} {}: {what}",
                            m.name
                        ))
                    };
                    let Some(set) = sets.iter().find(|x| x.name == m.name) else {
                        fail("not in the team".into());
                        continue;
                    };
                    if !species_ok(m.species, set.species) {
                        fail(format!(
                            "species {} vs {}",
                            m.details_species,
                            dex::species(set.species).name
                        ));
                    }
                    if m.level != set.level {
                        fail(format!("level {} vs {}", m.level, set.level));
                    }
                    if illusion && set.ability != dex::ABILITY_ILLUSION {
                        continue;
                    }
                    let true_moves = &set.moves[..set.move_count as usize];
                    for mv in &m.moves {
                        c.moves += 1;
                        if !true_moves.contains(mv) {
                            fail(format!("move {}", name(*mv)));
                        }
                    }
                    if let Some(item) = m.starting_item {
                        c.starting_items += 1;
                        if item != set.item {
                            fail(format!(
                                "starting item {} vs {}",
                                name(item),
                                name(set.item)
                            ));
                        }
                    }
                    if let Some(a) = m.starting_ability {
                        c.starting_abilities += 1;
                        if a != set.ability {
                            fail(format!(
                                "starting ability {} vs {}",
                                name(a),
                                name(set.ability)
                            ));
                        }
                    }
                    if let Some(tt) = m.tera_type {
                        c.tera_types += 1;
                        if tt != set.tera_type {
                            fail(format!("tera {:?} vs {:?}", tt, set.tera_type));
                        }
                    }
                }
            }
        }
    }
    eprintln!("{c:?}");
    for f in failures.iter().take(40) {
        eprintln!("{f}");
    }
    assert!(
        failures.is_empty(),
        "{} soundness violations",
        failures.len()
    );
    assert!(c.moves > 0 && c.starting_items > 0 && c.starting_abilities > 0 && c.tera_types > 0);
    assert!(
        n < 1000 || c.replaced > 0,
        "no Illusion reveal exercised: {c:?}"
    );
}
