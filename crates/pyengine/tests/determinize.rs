mod common;

use _numbrion::{env::SamplerParams, game::Game, pool::SplitMix};
use common::*;

/// `Game::replace_hidden_set` (Python `Battle.replace_hidden_set`): a world whose never-revealed opponent
/// sets were swapped equals a battle built from the swapped packed teams and the original seed that replays
/// the same decisions, and both play out identically afterwards (pool teams carry explicit genders, so the
/// same-seed reference applies). Revealed Pokemon are refused.
#[test]
fn swapped_world_matches_replayed_construction() {
    let pool = test_pool();
    let mut rng = SplitMix::new(77);
    let p = SamplerParams::default();
    let (mut worlds, mut swaps, mut refused) = (0, 0, 0);
    for _ in 0..n_battles(20) {
        let mut g = new_game(&pool, &mut rng, true);
        let mut history: Vec<Vec<(usize, usize, usize)>> = Vec::new();
        let target = rng.below(12);
        while !g.ended() && history.len() < target {
            history.push(random_step_codes(&mut g, &mut rng, &p, None));
        }
        if g.ended() {
            continue;
        }
        let viewer = rng.below(2);
        let mut world = g.duplicate();
        let hidden = world.hidden_team_indices(viewer);
        for index in (0..6).filter(|i| !hidden.contains(i)) {
            let e = world.replace_hidden_set(viewer, index, "Pikachu|||static|thunderbolt|||M|||50|").unwrap_err();
            assert!(e.contains("revealed") || e.contains("BattleStart"), "{e}");
            refused += 1;
        }
        let mut swapped_here = 0;
        for &index in &hidden {
            let donor = pool.get(rng.below(pool.len()));
            let set = donor.split(']').nth(rng.below(6)).unwrap().to_owned();
            match world.replace_hidden_set(viewer, index, &set) {
                Ok(()) => swapped_here += 1,
                Err(e) => assert!(e.contains("BattleStart") || e.contains("gender"), "unexpected refusal: {e}"),
            }
        }
        assert_eq!(world.same_origin(&g), swapped_here == 0);
        swaps += swapped_here;
        worlds += 1;
        let [p1, p2] = world.packed_teams().clone();
        let mut reference = Game::new(g.initial_seed(), p1, p2, g.names().clone(), true, true).unwrap();
        reference.start().unwrap();
        for codes in &history {
            apply_codes(&mut reference, codes);
        }
        assert_eq!(reference.signature(), world.signature());
        let seed = rng.battle_seed();
        reference.reseed(seed);
        world.reseed(seed);
        reference.drain_omni();
        for s in 0..2 {
            reference.drain_player(s);
        }
        while !world.ended() {
            let codes = random_step_codes(&mut world, &mut rng, &p, None);
            apply_codes(&mut reference, &codes);
            assert_eq!(reference.signature(), world.signature());
            assert_eq!(reference.drain_omni(), world.drain_omni());
            for s in 0..2 {
                assert_eq!(reference.drain_player(s), world.drain_player(s));
            }
        }
    }
    eprintln!("{worlds} worlds, {swaps} swaps, {refused} refusals");
    assert!(swaps > 10 && refused > 10);
}
