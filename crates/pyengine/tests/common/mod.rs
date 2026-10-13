#![allow(dead_code)]

use _numbrion::{
    action::{TARGET_AUTO, move_action},
    env::{SamplerParams, sample_from},
    game::Game,
    mask::SideActions,
    pool::{SplitMix, TeamPool},
};
use engine::ids::EffectId;

pub fn test_pool() -> TeamPool {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/pool-400.txt.gz");
    TeamPool::load(&[path], None).expect("test pool")
}

/// Number of battles a test plays: `NUMBRION_TEST_BATTLES` or the given default.
pub fn n_battles(default: usize) -> usize {
    std::env::var("NUMBRION_TEST_BATTLES").ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub fn new_game_with(pool: &TeamPool, i: usize, j: usize, seed: [u16; 4], log: bool) -> Option<Game> {
    let mut g = Game::new(seed, pool.get(i).clone(), pool.get(j).clone(), ["Alice".into(), "Bob".into()], log, true).ok()?;
    g.start().expect("start");
    Some(g)
}

pub fn new_game(pool: &TeamPool, rng: &mut SplitMix, log: bool) -> Game {
    for _ in 0..100 {
        let (i, j) = (rng.below(pool.len()), rng.below(pool.len()));
        let seed = rng.battle_seed();
        if let Some(g) = new_game_with(pool, i, j, seed, log) {
            return g;
        }
    }
    panic!("no usable team pair");
}

/// The code that uses the (untargeted) move `want` from slot `k`, if the mask allows it.
fn favourite(a: &SideActions, k: usize, mask: u64, want: Option<EffectId>) -> Option<usize> {
    let want = want?;
    let ls = &a.legal.slots[k];
    ls.moves[..ls.move_count as usize]
        .iter()
        .find(|m| m.move_id == want && m.move_slot != 255)
        .map(|m| move_action(m.move_slot, TARGET_AUTO, false))
        .filter(|&c| mask >> c & 1 == 1)
}

/// Submit random legal actions for every side that has to act; returns the number of sides that acted.
pub fn random_step(g: &mut Game, rng: &mut SplitMix, p: &SamplerParams) -> usize {
    random_step_prefer(g, rng, p, None)
}

/// Like `random_step`, but with probability 0.9 uses the given move whenever it is legal.
pub fn random_step_prefer(g: &mut Game, rng: &mut SplitMix, p: &SamplerParams, prefer: Option<EffectId>) -> usize {
    random_step_codes(g, rng, p, prefer).len()
}

/// Apply the same `(side, a0, a1)` decisions to another copy of the battle.
pub fn apply_codes(g: &mut Game, codes: &[(usize, usize, usize)]) {
    for &(s, a0, a1) in codes {
        let acts = g.side_actions(s);
        let ch = acts.choices(a0, a1, &g.party(s)).expect("same decision is legal in the copy");
        g.choose_typed(s, &ch).unwrap_or_else(|e| panic!("copy rejected ({a0},{a1}): {e}"));
    }
}

/// Like `random_step_prefer`, returning the decisions taken.
pub fn random_step_codes(g: &mut Game, rng: &mut SplitMix, p: &SamplerParams, prefer: Option<EffectId>) -> Vec<(usize, usize, usize)> {
    let mut n = Vec::new();
    let party = [g.party(0), g.party(1)];
    // Randomize which side submits first: it must not matter.
    let order = if rng.below(2) == 0 { [0, 1] } else { [1, 0] };
    let acts = [g.side_actions(0), g.side_actions(1)];
    for s in order {
        let a = &acts[s];
        if !a.needs_action() {
            continue;
        }
        let m0 = a.mask0();
        assert_ne!(m0, 0, "side {s} needs an action but has no legal code");
        let a0 = match favourite(a, 0, m0, prefer) {
            Some(c) if rng.unit() < 0.9 => c,
            _ => sample_from(m0, rng, p),
        };
        let m1 = a.mask1(a0);
        assert_ne!(m1, 0);
        let a1 = match favourite(a, 1, m1, prefer) {
            Some(c) if rng.unit() < 0.9 => c,
            _ => sample_from(m1, rng, p),
        };
        let ch = a.choices(a0, a1, &party[s]).expect("legal pair has engine choices");
        g.choose_typed(s, &ch).unwrap_or_else(|e| panic!("masked-legal ({a0},{a1}) rejected: {e}"));
        n.push((s, a0, a1));
    }
    assert!(!n.is_empty(), "no side had a pending request");
    n
}
