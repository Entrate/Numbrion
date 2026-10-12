mod common;

use _numbrion::{
    env::{SamplerParams, sample_from},
    game::Game,
    pool::SplitMix,
    search::{GridExecutor, Options, Seeds, Stop},
    search_grid::{LeafArrays, LeafOptions, PARTNER_FLOATS_PER_ROW, PARTNER_IDS_PER_ROW, SearchGrid, StateInput},
};
use common::*;
use encoder::{
    Dex,
    encode::{
        ACTION_PER_ROW, Encoder, FAINTED_PER_ROW, FIELD_PER_ROW, FLOATS_PER_ROW, FOE_MATCH_PER_ROW, GUARD_PER_ROW,
        IDS_PER_ROW, Observation, Outputs, TOKEN_FEATURES_PER_ROW,
    },
    tracker::Tracker,
};
use std::{path::Path, sync::Arc};

fn dex() -> Arc<Dex> {
    Dex::shared(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json")).expect("dex.json")
}

/// The output arrays of a 2-row `Encoder`.
#[derive(Clone)]
struct Rows {
    ids: Vec<i64>,
    floats: Vec<f32>,
    field: Vec<f32>,
    foe_match: Vec<i64>,
    fainted: Vec<f32>,
    guard: Vec<bool>,
    action: Vec<f32>,
    token: Vec<f32>,
}

impl Rows {
    fn new() -> Rows {
        Rows {
            ids: vec![0; 2 * IDS_PER_ROW],
            floats: vec![0.0; 2 * FLOATS_PER_ROW],
            field: vec![0.0; 2 * FIELD_PER_ROW],
            foe_match: vec![0; 2 * FOE_MATCH_PER_ROW],
            fainted: vec![0.0; 2 * FAINTED_PER_ROW],
            guard: vec![false; 2 * GUARD_PER_ROW],
            action: vec![0.0; 2 * ACTION_PER_ROW],
            token: vec![0.0; 2 * TOKEN_FEATURES_PER_ROW],
        }
    }

    /// `encode_batch` of both sides' (request, lines).
    fn encode(enc: &mut Encoder, obs: &[(Option<String>, Vec<String>); 2]) -> Rows {
        let mut r = Rows::new();
        let views: Vec<_> = obs.iter().map(|(req, log)| Observation { battle_id: 0, request: req.as_deref(), log }).collect();
        let mut out = Outputs { ids: &mut r.ids, floats: &mut r.floats, field: &mut r.field, foe_match: &mut r.foe_match,
            fainted: &mut r.fainted, guard: &mut r.guard, action_features: &mut r.action, token_features: &mut r.token };
        enc.encode_batch(&views, &mut out).expect("encode_batch");
        r
    }
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

fn observe(g: &mut Game) -> [(Option<String>, Vec<String>); 2] {
    [0, 1].map(|s| (g.request_json(s), g.drain_player(s)))
}

/// A root reached by random play, with the trackers of an encoder fed every boundary's observations.
struct Root {
    game: Game,
    trackers: [Tracker; 2],
}

fn random_root(dex: &Arc<Dex>, rng: &mut SplitMix, late: bool) -> Option<Root> {
    let pool = test_pool();
    let p = SamplerParams { switch_prob: 0.2, tera_prob: 0.2 };
    let mut g = new_game(&pool, rng, true);
    let mut enc = Encoder::new(dex.clone(), 2);
    Rows::encode(&mut enc, &observe(&mut g));
    let steps = if late { 200 } else { rng.below(25) };
    let mut boundaries = Vec::new();
    for _ in 0..steps {
        random_step(&mut g, rng, &p);
        if g.ended() {
            break;
        }
        Rows::encode(&mut enc, &observe(&mut g));
        boundaries.push((g.duplicate(), [enc.tracker(0).clone(), enc.tracker(1).clone()]));
    }
    // Late roots: one of the last boundaries before the end (cells there often end the battle).
    let pick = if late { boundaries.len().checked_sub(1 + rng.below(2))? } else { boundaries.len().checked_sub(1)? };
    let (game, trackers) = boundaries.swap_remove(pick);
    Some(Root { game, trackers })
}

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

fn same_arrays(a: &LeafArrays, b: &LeafArrays) {
    assert_eq!(a.eval_side, b.eval_side);
    assert_eq!(a.ids, b.ids);
    assert_eq!(bits(&a.floats), bits(&b.floats));
    assert_eq!(bits(&a.field), bits(&b.field));
    assert_eq!(bits(&a.token_features), bits(&b.token_features));
    assert_eq!(a.partner_ids, b.partner_ids);
    assert_eq!(bits(&a.partner_floats), bits(&b.partner_floats));
    assert_eq!(a.action_features.as_deref().map(bits), b.action_features.as_deref().map(bits));
    assert_eq!(a.guard, b.guard);
    assert_eq!((&a.grid.ended, &a.grid.winner, &a.grid.turn), (&b.grid.ended, &b.grid.winner, &b.grid.turn));
    assert_eq!((&a.grid.needs_action, &a.grid.request_kind, &a.grid.mask0), (&b.grid.needs_action, &b.grid.request_kind, &b.grid.mask0));
}

/// Fused leaves on 1 and 3 threads (workers dirty from earlier calls, several worlds per call, every seeding
/// mode) against GridExecutor's logs and requests encoded by a 2-row `Encoder` restored from the root trackers.
#[test]
fn fused_leaves_match_encode_batch() {
    let dex = dex();
    let mut rng = SplitMix::new(77);
    let mut sg1 = SearchGrid::new(dex.clone(), 1).unwrap();
    let mut sg3 = SearchGrid::new(dex.clone(), 3).unwrap();
    let mut ex = GridExecutor::new(2).unwrap();
    let (mut cells, mut ended, mut other, mut single) = (0, 0, 0, 0);
    for b in 0..n_battles(16) {
        let roots: Vec<Root> = (0..1 + rng.below(3)).filter_map(|w| random_root(&dex, &mut rng, (b + w) % 2 == 0)).collect();
        if roots.is_empty() {
            continue;
        }
        let mut actions = Vec::new();
        let mut world = Vec::new();
        for (k, r) in roots.iter().enumerate() {
            for _ in 0..6 {
                actions.extend(random_codes(&r.game, &mut rng));
                world.push(k as u32);
            }
        }
        let n = world.len();
        let seeds = match b % 3 {
            0 => Seeds::Root,
            1 => Seeds::Fixed(vec![rng.battle_seed()]),
            _ => Seeds::Fixed((0..n).map(|_| rng.battle_seed()).collect()),
        };
        let games: Vec<&Game> = roots.iter().map(|r| &r.game).collect();
        let pairs: Vec<[&Tracker; 2]> = roots.iter().map(|r| [&r.trackers[0], &r.trackers[1]]).collect();
        let prefer = b % 2;
        let opts = LeafOptions { stop: if b % 4 == 3 { Stop::Turn } else { Stop::Boundary }, masks: true, action_features: true, guard: true };
        let out = sg1.run(&games, &world, &actions, &seeds, &pairs, prefer, &opts).unwrap();
        same_arrays(&out, &sg3.run(&games, &world, &actions, &seeds, &pairs, prefer, &opts).unwrap());
        let slow_opts = Options { stop: opts.stop, requests: true, masks: true, logs: true, keep_leaves: false };
        let leaves = ex.run(&games, &world, &actions, &seeds, &slow_opts).unwrap();
        for (i, leaf) in leaves.iter().enumerate() {
            let r = &roots[world[i] as usize];
            let want = [prefer, 1 - prefer].into_iter().find(|&s| leaf.needs[s]).map_or(-1, |s| s as i8);
            assert_eq!(out.eval_side[i], want, "cell {i}");
            assert_eq!(out.grid.ended[i] != 0, leaf.ended);
            cells += 1;
            single += usize::from(leaf.needs[0] != leaf.needs[1]);
            if want < 0 {
                ended += 1;
                assert!(leaf.ended && out.ids[i * IDS_PER_ROW..(i + 1) * IDS_PER_ROW].iter().all(|&x| x == 0));
                continue;
            }
            other += usize::from(want as usize != prefer);
            let mut enc = Encoder::new(dex.clone(), 2);
            enc.restore_tracker(0, &r.trackers[0]).unwrap();
            enc.restore_tracker(1, &r.trackers[1]).unwrap();
            let obs = [0, 1].map(|s| (leaf.requests[s].clone(), leaf.logs[s].clone()));
            let rows = Rows::encode(&mut enc, &obs);
            let (e, p) = (want as usize, 1 - want as usize);
            assert_eq!(out.ids[i * IDS_PER_ROW..][..IDS_PER_ROW], rows.ids[e * IDS_PER_ROW..][..IDS_PER_ROW], "cell {i}");
            assert_eq!(bits(&out.floats[i * FLOATS_PER_ROW..][..FLOATS_PER_ROW]), bits(&rows.floats[e * FLOATS_PER_ROW..][..FLOATS_PER_ROW]));
            assert_eq!(bits(&out.field[i * FIELD_PER_ROW..][..FIELD_PER_ROW]), bits(&rows.field[e * FIELD_PER_ROW..][..FIELD_PER_ROW]));
            let t = TOKEN_FEATURES_PER_ROW;
            assert_eq!(bits(&out.token_features[i * t..][..t]), bits(&rows.token[e * t..][..t]));
            let a = ACTION_PER_ROW;
            assert_eq!(bits(&out.action_features.as_ref().unwrap()[i * a..][..a]), bits(&rows.action[e * a..][..a]));
            assert_eq!(out.guard.as_ref().unwrap()[i * GUARD_PER_ROW..][..GUARD_PER_ROW], rows.guard[e * GUARD_PER_ROW..][..GUARD_PER_ROW]);
            assert_eq!(out.partner_ids[i * PARTNER_IDS_PER_ROW..][..PARTNER_IDS_PER_ROW], rows.ids[p * IDS_PER_ROW..][..PARTNER_IDS_PER_ROW]);
            assert_eq!(bits(&out.partner_floats[i * PARTNER_FLOATS_PER_ROW..][..PARTNER_FLOATS_PER_ROW]),
                       bits(&rows.floats[p * FLOATS_PER_ROW..][..PARTNER_FLOATS_PER_ROW]));
        }
    }
    eprintln!("{cells} cells: {ended} ended, {single} with one side acting, {other} evaluated from the other side");
    assert!(cells > 0 && ended > 0 && single > 0 && other > 0);
}

/// `Game::request_tree` equals the parsed request JSON at every boundary of many battles (move, replacement,
/// wait and Revival Blessing requests, Tera, locked and disabled moves), and after rejected choices that update
/// a request with hidden information (`update`, a late `trapped`, `disabledSource`).
#[test]
fn request_tree_equals_parsed_request_json() {
    let pool = test_pool();
    let revival: Vec<_> = (0..pool.len()).filter(|&i| pool.get(i).contains("revivalblessing")).collect();
    let mut rng = SplitMix::new(31);
    let p = SamplerParams { switch_prob: 0.15, tera_prob: 0.2 };
    let check = |g: &Game, counts: &mut [usize; 4]| {
        for s in 0..2 {
            let text = g.request_json(s);
            let parsed = text.as_deref().map(|t| encoder::json::Json::parse(t).expect("request JSON"));
            assert_eq!(g.request_tree(s), parsed, "side {s}: {text:?}");
            if let Some(t) = &text {
                counts[0] += 1;
                counts[1] += usize::from(t.contains("\"update\":true"));
                counts[2] += usize::from(t.contains("disabledSource") || t.contains("\"trapped\":true"));
                counts[3] += usize::from(t.contains("forceSwitch"));
            }
        }
    };
    let mut counts = [0usize; 4];
    for b in 0..n_battles(60) {
        let mut g = if b % 4 == 0 {
            let (i, j) = (revival[rng.below(revival.len())], revival[rng.below(revival.len())]);
            match new_game_with(&pool, i, j, rng.battle_seed(), b % 2 == 0) {
                Some(g) => g,
                None => continue,
            }
        } else {
            new_game(&pool, &mut rng, b % 2 == 0)
        };
        let prefer = (b % 4 == 0).then_some(engine::dex::MOVE_REVIVALBLESSING);
        while !g.ended() {
            check(&g, &mut counts);
            // Blind text choices on a copy: a rejection can re-send an updated request.
            for s in 0..2 {
                if g.side_actions(s).needs_action() {
                    let mut c = g.duplicate();
                    let choice = ["switch 3, move 1", "move 1, switch 4", "move 2 1, move 3 2", "switch 5"][rng.below(4)];
                    if c.choose(s, choice).is_err() {
                        check(&c, &mut counts);
                    }
                }
            }
            random_step_prefer(&mut g, &mut rng, &p, prefer);
        }
        check(&g, &mut counts);
    }
    eprintln!("{} requests ({} updated, {} with disabledSource/trapped, {} replacement)", counts[0], counts[1], counts[2], counts[3]);
    assert!(counts[0] > 0 && counts[3] > 0);
}

#[test]
fn encode_states_match_encode_batch_and_validate() {
    let dex = dex();
    let mut rng = SplitMix::new(5);
    let sg = SearchGrid::new(dex.clone(), 2).unwrap();
    let root = (0..10).find_map(|_| random_root(&dex, &mut rng, false)).expect("a root");
    let requests = [root.game.request_json(0), root.game.request_json(1)];
    let lines = vec!["|-boost|p1a: Nobody|atk|1".to_string()];
    let states: Vec<StateInput<'_, String>> = (0..4)
        .map(|k| StateInput { tracker: &root.trackers[k % 2], request: requests[k % 2].as_deref(), lines: if k < 2 { &[] } else { &lines } })
        .collect();
    let out = sg.encode_states(&states, true, true).unwrap();
    for (k, st) in states.iter().enumerate() {
        let s = k % 2;
        let mut enc = Encoder::new(dex.clone(), 2);
        enc.restore_tracker(s, st.tracker).unwrap();
        let mut obs: [(Option<String>, Vec<String>); 2] = Default::default();
        obs[s] = (requests[s].clone(), st.lines.to_vec());
        let rows = Rows::encode(&mut enc, &obs);
        assert_eq!(out.ids[k * IDS_PER_ROW..][..IDS_PER_ROW], rows.ids[s * IDS_PER_ROW..][..IDS_PER_ROW]);
        assert_eq!(bits(&out.floats[k * FLOATS_PER_ROW..][..FLOATS_PER_ROW]), bits(&rows.floats[s * FLOATS_PER_ROW..][..FLOATS_PER_ROW]));
        let a = ACTION_PER_ROW;
        assert_eq!(bits(&out.action_features.as_ref().unwrap()[k * a..][..a]), bits(&rows.action[s * a..][..a]));
        let t = TOKEN_FEATURES_PER_ROW;
        assert_eq!(bits(&out.token_features[k * t..][..t]), bits(&rows.token[s * t..][..t]));
    }
    // Validation.
    let mut sg = sg;
    let pair = [&root.trackers[0], &root.trackers[1]];
    let swapped = [&root.trackers[1], &root.trackers[0]];
    let codes = random_codes(&root.game, &mut rng);
    let opts = LeafOptions::default();
    let run = |sg: &mut SearchGrid, pairs: &[[&Tracker; 2]], prefer: usize, codes: &[i32]| {
        sg.run(&[&root.game], &[], codes, &Seeds::Root, pairs, prefer, &opts).map(|_| ()).map_err(|e| e.to_string())
    };
    assert!(run(&mut sg, &[pair], 0, &codes).is_ok());
    assert!(run(&mut sg, &[swapped], 0, &codes).unwrap_err().contains("trackers[0][0] tracks side 1"));
    assert!(run(&mut sg, &[pair, pair], 0, &codes).unwrap_err().contains("one pair per root"));
    assert!(run(&mut sg, &[pair], 2, &codes).unwrap_err().contains("eval_side"));
    let mut bad = codes;
    bad[0] = 46;
    bad[1] = 46;
    if root.game.side_actions(0).needs_action() && !root.game.side_actions(0).is_legal(46, 46) {
        assert!(run(&mut sg, &[pair], 0, &bad).unwrap_err().contains("cell 0 side 0"));
    }
    let mut quiet = Game::new([1, 2, 3, 4], test_pool().get(0).clone(), test_pool().get(1).clone(), ["A".into(), "B".into()], false, false).unwrap();
    quiet.start().unwrap();
    let codes = random_codes(&quiet, &mut rng);
    let e = sg.run(&[&quiet], &[], &codes, &Seeds::Root, &[pair], 0, &opts).map(|_| ()).unwrap_err().to_string();
    assert!(e.contains("log=True"), "{e}");
}
