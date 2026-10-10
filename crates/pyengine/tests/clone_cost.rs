mod common;
use _numbrion::{game::Game, pool::SplitMix};
use common::*;
use std::time::Instant;

#[test]
fn clone_cost() {
    let pool = test_pool();
    let mut rng = SplitMix::new(5);
    let g = new_game(&pool, &mut rng, false);
    let n = 300;
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(g.duplicate());
    }
    eprintln!("duplicate: {:?} each", t.elapsed() / n);
    let mut h = g.duplicate();
    let t = Instant::now();
    for _ in 0..n * 100 {
        h.copy_state_from(&g);
    }
    eprintln!("copy_state_from: {:?} each", t.elapsed() / (n * 100));
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(
            Game::new(g.initial_seed(), g.packed_teams()[0].clone(), g.packed_teams()[1].clone(), ["a".into(), "b".into()], false, false)
                .unwrap(),
        );
    }
    eprintln!("Game::new: {:?} each", t.elapsed() / n);
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(engine::teams::TeamDef::unpack(&g.packed_teams()[0]).unwrap());
    }
    eprintln!("TeamDef::unpack: {:?} each", t.elapsed() / n);
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(engine::state::BattleState::empty([1, 2, 3, 4]));
    }
    eprintln!("BattleState::empty: {:?} each", t.elapsed() / n);
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(engine::event::Scratch::default());
    }
    eprintln!("Scratch::default: {:?} each", t.elapsed() / n);
    eprintln!("size_of Battle<NoLog> = {}", std::mem::size_of::<engine::Battle<engine::log::NoLog>>());
}
