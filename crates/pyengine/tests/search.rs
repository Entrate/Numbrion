mod common;

use _numbrion::{
    env::{SamplerParams, sample_from},
    game::Game,
    mask::Req,
    pool::SplitMix,
    search::{GridError, GridExecutor, Leaf, Options, Seeds, Stop},
};
use common::*;

/// Everything a leaf reports, plus the leaf battle's own signature (PRNG included).
type Summary = (bool, i8, i32, u8, [u8; 2], [bool; 2], [u64; 2], [u64; 2], [Option<String>; 2], [Vec<String>; 2],
    Option<(u32, [u16; 4], bool, [Option<String>; 2])>);

fn summary(l: &Leaf) -> Summary {
    (l.ended, l.winner, l.turn, l.steps, l.req, l.needs, l.mask0, l.mask1_any, l.requests.clone(), l.logs.clone(),
        l.game.as_ref().map(|g| g.signature()))
}

/// The cell computed the slow, obvious way: a fresh copy of the root, reseeded, both choices, then (for
/// `Stop::Turn`) Showdown's `default` for every replacement request.
fn naive(root: &Game, codes: &[i32], seed: Option<[u16; 4]>, opts: &Options) -> (Summary, Game) {
    let mut g = root.duplicate();
    if let Some(s) = seed {
        g.reseed(s);
    }
    for s in 0..2 {
        let a = root.side_actions(s);
        if a.needs_action() {
            let ch = a.choices(codes[2 * s] as usize, codes[2 * s + 1] as usize, &root.party(s)).unwrap();
            g.choose_typed(s, &ch).unwrap();
        }
    }
    let mut steps = 1;
    while opts.stop == Stop::Turn && !g.ended() && (0..2).any(|s| g.side_actions(s).req == Req::Switch) {
        let acts = [g.side_actions(0), g.side_actions(1)];
        for s in 0..2 {
            if acts[s].req == Req::Switch {
                g.choose(s, "default").unwrap();
            }
        }
        steps += 1;
    }
    let acts = [g.side_actions(0), g.side_actions(1)];
    let m0 = acts.map(|a| if opts.masks && a.needs_action() { a.mask0() } else { 0 });
    let leaf = Leaf {
        ended: g.ended(),
        winner: g.outcome().and_then(|o| o.winner).map_or(-1, |w| w as i8),
        turn: g.turn() as i32,
        steps,
        req: acts.map(|a| a.req.code()),
        needs: acts.map(|a| a.needs_action()),
        mask0: m0,
        mask1_any: [acts[0].mask1_any(m0[0]), acts[1].mask1_any(m0[1])],
        requests: if opts.requests { [g.request_json(0), g.request_json(1)] } else { [None, None] },
        logs: if opts.logs { [g.drain_player(0), g.drain_player(1)] } else { [vec![], vec![]] },
        game: opts.keep_leaves.then(|| Box::new(g.duplicate())),
    };
    (summary(&leaf), g)
}

/// A random mask-legal pair for every side that has to act at `root`, -1 elsewhere.
fn random_codes(root: &Game, rng: &mut SplitMix) -> [i32; 4] {
    let p = SamplerParams { switch_prob: 0.25, tera_prob: 0.3 };
    let mut out = [-1; 4];
    for s in 0..2 {
        let a = root.side_actions(s);
        if a.needs_action() {
            let a0 = sample_from(a.mask0(), rng, &p);
            out[2 * s] = a0 as i32;
            out[2 * s + 1] = sample_from(a.mask1(a0), rng, &p) as i32;
        }
    }
    out
}

#[derive(Default, Debug)]
struct Counts {
    grids: usize,
    cells: usize,
    switch_roots: usize,
    switch_leaves: usize,
    pivots: usize,
    faints: usize,
    revivals: usize,
    ended: usize,
    multi_step: usize,
}

/// Run grids from random boundaries of many battles (several worlds per call, mixed seeding modes) on a
/// 1-thread and a 3-thread executor whose workers are dirty from earlier calls, and compare every cell with
/// a naive copy. Leaves must be identical everywhere and play on identically.
fn check_grids(stop: Stop, battles: usize, seed: u64) -> Counts {
    let pool = test_pool();
    let revival: Vec<_> = (0..pool.len()).filter(|&i| pool.get(i).contains("revivalblessing")).collect();
    let mut rng = SplitMix::new(seed);
    let p = SamplerParams::default();
    let mut ex1 = GridExecutor::new(1).unwrap();
    let mut ex3 = GridExecutor::new(3).unwrap();
    let mut c = Counts::default();
    for b in 0..battles {
        let log = b % 3 != 2;
        // A few independent battles at random boundaries serve as the worlds of one call.
        let mut games: Vec<Game> = Vec::new();
        for w in 0..1 + rng.below(3) {
            let rb = (b + w) % 4 == 0;
            let g = if rb {
                let (i, j) = (revival[rng.below(revival.len())], revival[rng.below(revival.len())]);
                new_game_with(&pool, i, j, rng.battle_seed(), log)
            } else {
                Some(new_game(&pool, &mut rng, log))
            };
            let Some(mut g) = g else { continue };
            let prefer = rb.then_some(engine::dex::MOVE_REVIVALBLESSING);
            for _ in 0..rng.below(30) {
                random_step_prefer(&mut g, &mut rng, &p, prefer);
                if g.ended() {
                    break;
                }
            }
            if !g.ended() {
                // Unread lines of the root must not leak into the leaves.
                games.push(g);
            }
        }
        if games.is_empty() {
            continue;
        }
        let n = 1 + rng.below(24);
        let world: Vec<u32> = (0..n).map(|_| rng.below(games.len()) as u32).collect();
        let actions: Vec<i32> = world.iter().flat_map(|&w| random_codes(&games[w as usize], &mut rng)).collect();
        let seeds = match b % 3 {
            0 => Seeds::Root,
            1 => Seeds::Fixed(vec![rng.battle_seed()]),
            _ => Seeds::Fixed((0..n).map(|_| rng.battle_seed()).collect()),
        };
        let opts = Options { stop, requests: true, masks: b % 2 == 0, logs: log, keep_leaves: b % 4 != 3 };
        let roots: Vec<&Game> = games.iter().collect();
        let got1 = ex1.run(&roots, &world, &actions, &seeds, &opts).unwrap();
        let mut got3 = ex3.run(&roots, &world, &actions, &seeds, &opts).unwrap();
        assert_eq!(got1.len(), n);
        c.grids += 1;
        for i in 0..n {
            let w = &games[world[i] as usize];
            let seed = match &seeds {
                Seeds::Root => None,
                Seeds::Fixed(s) => Some(s[if s.len() == 1 { 0 } else { i }]),
            };
            let (want, mut g) = naive(w, &actions[4 * i..4 * i + 4], seed, &opts);
            assert_eq!(summary(&got1[i]), want, "battle {b} cell {i}: 1-thread executor differs from a naive copy");
            assert_eq!(summary(&got3[i]), want, "battle {b} cell {i}: 3-thread executor differs from a naive copy");
            c.cells += 1;
            c.switch_roots += (0..2).any(|s| w.side_actions(s).req == Req::Switch) as usize;
            c.switch_leaves += got1[i].req.contains(&2) as usize;
            let acts = [g.side_actions(0), g.side_actions(1)];
            if acts.iter().any(|a| a.req == Req::Switch) {
                let st = g.state();
                let active = st.sides.iter().flat_map(|s| s.active).filter(|m| *m != engine::ids::MonId::NONE);
                if acts.iter().any(|a| a.revival.contains(&true)) {
                    c.revivals += 1;
                } else if active.clone().any(|m| st.pokemon[m.0 as usize].hp == 0) {
                    c.faints += 1;
                } else {
                    c.pivots += 1;
                }
            }
            c.ended += got1[i].ended as usize;
            c.multi_step += (got1[i].steps > 1) as usize;
            if stop == Stop::Turn {
                assert!(got1[i].ended || !got1[i].req.contains(&2), "turn mode stopped at a replacement request");
            }
            // The returned leaf is an independent battle that plays on exactly like the naive copy.
            if let Some(leaf) = got3[i].game.as_mut() {
                for _ in 0..3 {
                    if g.ended() {
                        break;
                    }
                    let codes = random_step_codes(&mut g, &mut rng, &p, None);
                    apply_codes(leaf, &codes);
                    assert_eq!(leaf.signature(), g.signature(), "battle {b} cell {i}: leaf diverged");
                    if log {
                        for s in 0..2 {
                            assert_eq!(leaf.drain_player(s), g.drain_player(s));
                        }
                    }
                }
            }
        }
        // The roots were only read.
        for g in &games {
            assert!(!g.ended() && g.started());
        }
    }
    c
}

#[test]
fn grid_cells_match_naive_copies_on_any_thread_count() {
    let c = check_grids(Stop::Boundary, n_battles(60), 5);
    eprintln!("boundary: {c:?}");
    assert!(c.grids > 30 && c.switch_roots > 0 && c.ended > 0);
    assert!(c.pivots > 0 && c.faints > 0 && c.revivals > 0 && c.switch_leaves == c.pivots + c.faints + c.revivals);
    assert_eq!(c.multi_step, 0);
}

#[test]
fn turn_mode_answers_replacements_with_the_default_choice() {
    let c = check_grids(Stop::Turn, n_battles(40), 6);
    eprintln!("turn: {c:?}");
    assert!(c.grids > 20 && c.multi_step > 0);
    assert_eq!(c.switch_leaves + c.pivots + c.faints + c.revivals, 0);
}

#[test]
fn bad_inputs_are_rejected_before_anything_runs() {
    let pool = test_pool();
    let mut rng = SplitMix::new(9);
    let mut ex = GridExecutor::new(2).unwrap();
    let root = new_game(&pool, &mut rng, false);
    let text = new_game(&pool, &mut rng, true);
    let ok = random_codes(&root, &mut rng);
    let opts = Options::default();
    let input = |r: Result<Vec<Leaf>, GridError>| match r {
        Err(GridError::Input(e)) => e,
        Err(e) => panic!("expected an input error, got {e}"),
        Ok(_) => panic!("expected an input error"),
    };
    let mut bad = ok;
    bad[0] = 46;
    bad[1] = 46;
    let e = input(ex.run(&[&root], &[], &[ok, bad].concat(), &Seeds::Root, &opts));
    assert!(e.contains("cell 1 side 0") && e.contains("is not legal"), "{e}");
    assert!(input(ex.run(&[&root], &[1], &ok, &Seeds::Root, &opts)).contains("out of range"));
    assert!(input(ex.run(&[&root], &[], &ok, &Seeds::Fixed(vec![[1, 2, 3, 4]; 2]), &opts)).contains("seeds"));
    assert!(input(ex.run(&[&root], &[], &ok[..3], &Seeds::Root, &opts)).contains("[n, 2, 2]"));
    assert!(input(ex.run(&[&root, &text], &[0], &ok, &Seeds::Root, &opts)).contains("log mode"));
    assert!(input(ex.run(&[&root], &[], &ok, &Seeds::Root, &Options { logs: true, ..opts })).contains("log=True"));
    let unstarted = Game::new([1, 2, 3, 4], pool.get(0).clone(), pool.get(1).clone(), ["A".into(), "B".into()], false, false).unwrap();
    assert!(input(ex.run(&[&unstarted], &[], &ok, &Seeds::Root, &opts)).contains("not been started"));
    // A root where one side has already chosen: the other side's choice would run the turn.
    let mut half = root.duplicate();
    let a = half.side_actions(0);
    half.choose_typed(0, &a.choices(ok[0] as usize, ok[1] as usize, &half.party(0)).unwrap()).unwrap();
    assert!(input(ex.run(&[&half], &[], &ok, &Seeds::Root, &opts)).contains("already chosen"));
    // Still usable, and an empty grid is fine.
    assert_eq!(ex.run(&[&root], &[], &ok, &Seeds::Root, &opts).unwrap().len(), 1);
    assert!(ex.run(&[&root], &[], &[], &Seeds::Root, &opts).unwrap().is_empty());
}
