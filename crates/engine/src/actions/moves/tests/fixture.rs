//! Shared battle fixtures: a two-mon-per-side doubles battle with hand-set actives, as the
//! other query probes do (construction stops before start, so nothing is switched in).
use crate::{
    Battle,
    dex::{self},
    ids::*,
    log::NoLog,
    prng::Prng,
    state::mon_flags,
};

pub const TEAM: &str = "Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric]\
Raichu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric";
/// Active order of every vector: p1a, p1b, p2a, p2b.
pub const ACTIVES: [MonId; 4] = [MonId(0), MonId(1), MonId(6), MonId(7)];

/// Three states per active, as in the oracle probe: 0 alive, 1 hp 0 but not yet fainted, 2 fainted.
pub fn battle_with(state: [u8; 4]) -> Battle<NoLog> {
    let mut b = Battle::new([1, 2, 3, 4], TEAM, TEAM).unwrap();
    for (i, &m) in ACTIVES.iter().enumerate() {
        let p = &mut b.state.pokemon[m.0 as usize];
        p.flags |= mon_flags::ACTIVE;
        p.flags &= !mon_flags::NEWLY_SWITCHED;
        p.position = (i % 2) as u8;
        p.hp = if state[i] == 0 { p.max_hp } else { 0 };
        if state[i] == 2 {
            p.flags |= mon_flags::FAINTED;
        }
    }
    b.state.sides[0].active = [ACTIVES[0], ACTIVES[1]];
    b.state.sides[1].active = [ACTIVES[2], ACTIVES[3]];
    b
}

pub fn reseed(b: &mut Battle<NoLog>) {
    b.state.prng = Prng::from_seed([1, 2, 3, 4]);
}

pub fn move_id(name: &str) -> EffectId {
    dex::lookup(EffectKind::Move, name).unwrap_or_else(|| panic!("unknown move {name}"))
}

/// Seed rendered like Showdown's `prng.getSeed()`.
pub fn seed_string(b: &Battle<NoLog>) -> String {
    let s = b.seed();
    format!("{},{},{},{}", s[0], s[1], s[2], s[3])
}
