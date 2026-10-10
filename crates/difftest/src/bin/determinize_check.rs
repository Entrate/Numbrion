//! Determinization reference check on recorded battles (`Battle::replace_hidden_set`).
//!
//!   determinize_check <fixtures.jsonl[.gz]>... [--jobs N] [--rate P] [--max-per-battle K] [--seed S]
//!
//! Replays each fixture's recorded choice history (accepted and rejected calls). At sampled
//! decision boundaries it picks a viewer, swaps one of its opponent's never-revealed sets for a
//! set from another battle of the corpus, and requires (see tests/determinize.rs in the engine
//! for the contract):
//! 1. `from_team_defs(s0, resolved teams after the swap)` replaying the same history reproduces
//!    the original log, PRNG, turn and the viewer's request at every boundary, every call's
//!    accept/reject, and ends with the swapped battle's exact state, teams and requests;
//! 2. with an unchanged gender-draw count, construction from the fixture seed and the swapped
//!    packed inputs gives the same state too;
//! 3. swapped and reference battles then play out identically under the same random choices
//!    (logs, both requests, PRNG, outcome), half of them after a common reseed;
//! 4. every opponent Pokemon outside `hidden_mons` is refused without changing the battle.
//!
//! Exits nonzero on the first violation; prints counts otherwise.
use difftest::fixture::Fixture;
use engine::{
    Battle,
    battle::HiddenSetError,
    dex,
    ids::{MonId, SideId},
    log::TextLog,
    prng::Prng,
    state::choices::{ChoiceKind, RequestKind, SlotChoice},
    teams::{Gender, SetDef, TeamDef},
};
use std::{io::BufRead, sync::Arc};

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
        [x as u16, (x >> 16) as u16, (x >> 32) as u16, (x >> 48) as u16]
    }
}

fn needs_action(b: &B, side: SideId) -> bool {
    matches!(b.side_request_kind(side), RequestKind::Move | RequestKind::Switch)
        && b.legal_actions(side).slot_count > 0
}

/// Random legal joint choice: a switch with probability 0.15 when offered, else a move.
fn pick(b: &B, side: SideId, rng: &mut Rng) -> Vec<SlotChoice> {
    let legal = b.legal_actions(side);
    let party = b.state.sides[side.0 as usize].party;
    let options: Vec<(Vec<SlotChoice>, Vec<SlotChoice>)> = legal.slots
        [..legal.slot_count as usize]
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

fn observe(b: &B) -> ([u16; 4], u32, [Option<String>; 2], bool) {
    (b.seed(), b.turn(), [b.request_json(0), b.request_json(1)], b.state.ended)
}

fn draws_gender(set: &SetDef) -> bool {
    set.gender == Gender::Unspecified
        && !matches!(dex::species(set.species).gender, "M" | "F" | "N")
}

/// One recorded `choose` call: side, input, accepted.
type Call = (usize, String, bool);

#[derive(Default, Debug, Clone)]
struct Counts {
    battles: usize,
    boundaries: usize,
    swaps: usize,
    replacement_boundaries: usize,
    same_seed: usize,
    draw_count_changed: usize,
    gender_required: usize,
    battle_start: usize,
    refused_revealed: usize,
    refused_modified: usize,
    replayed_calls: usize,
    continued_boundaries: usize,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        self.battles += o.battles;
        self.boundaries += o.boundaries;
        self.swaps += o.swaps;
        self.replacement_boundaries += o.replacement_boundaries;
        self.same_seed += o.same_seed;
        self.draw_count_changed += o.draw_count_changed;
        self.gender_required += o.gender_required;
        self.battle_start += o.battle_start;
        self.refused_revealed += o.refused_revealed;
        self.refused_modified += o.refused_modified;
        self.replayed_calls += o.replayed_calls;
        self.continued_boundaries += o.continued_boundaries;
    }
}

struct Origin<'a> {
    fx: &'a Fixture,
    s0: [u16; 4],
    raw: [Arc<TeamDef>; 2],
}

impl Origin<'_> {
    fn names(&self) -> [String; 2] {
        [self.fx.name(0).to_owned(), self.fx.name(1).to_owned()]
    }
    fn build(&self, seed: [u16; 4], p1: &Arc<TeamDef>, p2: &Arc<TeamDef>) -> B {
        B::from_team_defs(seed, p1.clone(), p2.clone(), self.names(), TextLog::default()).unwrap()
    }
}

fn replay(b: &mut B, calls: &[Call], what: &str) {
    for (side, input, ok) in calls {
        assert_eq!(b.choose(*side, input).is_ok(), *ok, "{what}: call {input:?} by p{} changed outcome", side + 1);
    }
}

#[allow(clippy::too_many_arguments)]
fn check_boundary(
    o: &B,
    origin: &Origin<'_>,
    history: &[Vec<Call>],
    seen: &[([u16; 4], u32, [Option<String>; 2], bool)],
    pool: &[SetDef],
    rng: &mut Rng,
    c: &mut Counts,
) {
    c.boundaries += 1;
    let viewer = SideId(rng.below(2) as u8);
    let opp = 1 - viewer.0 as usize;
    let hidden = o.hidden_mons(viewer);
    let any_set = &pool[rng.below(pool.len())];
    let mut probe = o.clone();
    let before = format!("{:?}", probe.state);
    let own = o.state.sides[viewer.0 as usize].party[0];
    assert_eq!(probe.replace_hidden_set(viewer, own, any_set), Err(HiddenSetError::NotOpponent));
    for &m in &o.state.sides[opp].party[..o.state.sides[opp].pokemon_count as usize] {
        if hidden & (1 << m.0) != 0 {
            continue;
        }
        match probe.replace_hidden_set(viewer, m, any_set).unwrap_err() {
            HiddenSetError::Revealed => c.refused_revealed += 1,
            HiddenSetError::Modified => c.refused_modified += 1,
            HiddenSetError::BattleStartSpecies => c.battle_start += 1,
            e => panic!("unexpected refusal {e:?}"),
        }
    }
    assert_eq!(format!("{:?}", probe.state), before, "a refused swap changed the battle");
    let candidates: Vec<MonId> = (0..12).filter(|&i| hidden & (1 << i) != 0).map(MonId).collect();
    if candidates.is_empty() {
        return;
    }
    let mon = candidates[rng.below(candidates.len())];
    let index = mon.0 as usize % 6;
    let mut set = pool[rng.below(pool.len())].clone();
    if rng.chance(0.3) {
        set.gender = Gender::Unspecified;
    }
    let raw_old = &origin.raw[opp].sets[index];
    let mut r = o.clone();
    let mut result = r.replace_hidden_set(viewer, mon, &set);
    if result == Err(HiddenSetError::GenderRequired) {
        assert!(draws_gender(&set) && !draws_gender(raw_old));
        c.gender_required += 1;
        set.gender = if rng.chance(0.5) { Gender::Male } else { Gender::Female };
        result = r.replace_hidden_set(viewer, mon, &set);
    }
    match result {
        Ok(()) => {}
        Err(HiddenSetError::BattleStartSpecies) => {
            c.battle_start += 1;
            return;
        }
        Err(e) => panic!("hidden_mons promised a swap, got {e:?}"),
    }
    c.swaps += 1;
    c.replacement_boundaries +=
        usize::from((0..2).any(|s| o.side_request_kind(SideId(s)) == RequestKind::Switch));
    assert_eq!(r.revealed_mons(), o.revealed_mons());
    assert_eq!(r.seed(), o.seed());
    let mut swapped = o.teams().clone();
    Arc::make_mut(&mut swapped.sides[opp]).sets[index] = r.teams().sides[opp].sets[index].clone();
    assert_eq!(r.teams(), &swapped, "only the swapped set may change");

    // 1. Resolved reference.
    let t = r.teams().clone();
    let mut f = origin.build(origin.s0, &t.sides[0], &t.sides[1]);
    assert_eq!((f.seed(), f.teams()), (origin.s0, r.teams()));
    f.start().unwrap();
    for (k, calls) in history.iter().enumerate() {
        let got = observe(&f);
        let v = viewer.0 as usize;
        assert_eq!((got.0, got.1, &got.2[v]), (seen[k].0, seen[k].1, &seen[k].2[v]), "reference diverged at boundary {k}");
        replay(&mut f, calls, "reference");
        c.replayed_calls += calls.len();
    }
    assert_eq!(f.log.entries, o.log.entries, "reference history log differs");
    assert_eq!(format!("{:?}", f.state), format!("{:?}", r.state), "state differs from reference");
    assert_eq!(observe(&f), observe(&r));

    // 2. Same-seed reference.
    if draws_gender(raw_old) == draws_gender(&set) {
        c.same_seed += 1;
        let mut raw = origin.raw.clone();
        Arc::make_mut(&mut raw[opp]).sets[index] = set.clone();
        let mut g = origin.build(origin.fx.battle_seed, &raw[0], &raw[1]);
        assert_eq!(g.teams(), r.teams());
        g.start().unwrap();
        for calls in history {
            replay(&mut g, calls, "same-seed reference");
        }
        assert_eq!(g.log.entries, o.log.entries);
        assert_eq!(format!("{:?}", g.state), format!("{:?}", r.state), "same-seed state differs");
    } else {
        c.draw_count_changed += 1;
    }

    // 3. Identical future.
    if rng.chance(0.5) {
        let seed = rng.seed();
        r.state.prng = Prng::from_seed(seed);
        f.state.prng = Prng::from_seed(seed);
    }
    let offset = f.log.entries.len();
    let mut play = Rng(rng.next());
    for _ in 0..400 {
        assert_eq!(observe(&r), observe(&f));
        assert_eq!(r.log.entries[..], f.log.entries[offset..]);
        assert_eq!(r.outcome(), f.outcome());
        c.continued_boundaries += 1;
        if r.state.ended {
            break;
        }
        for side in [SideId(0), SideId(1)].into_iter().filter(|&s| needs_action(&r, s)).collect::<Vec<_>>() {
            let choice = pick(&r, side, &mut play);
            r.choose_typed(side, &choice).unwrap();
            f.choose_typed(side, &choice).unwrap();
        }
    }
}

fn run(fx: &Fixture, pool: &[SetDef], rng: &mut Rng, rate: f64, max_per_battle: usize, c: &mut Counts) {
    let raw = [Arc::new(TeamDef::unpack(&fx.teams[0]).unwrap()), Arc::new(TeamDef::unpack(&fx.teams[1]).unwrap())];
    let mut origin = Origin { fx, s0: [0; 4], raw };
    let mut o = origin.build(fx.battle_seed, &origin.raw[0], &origin.raw[1]);
    origin.s0 = o.seed();
    o.start().unwrap();
    let mut history: Vec<Vec<Call>> = vec![];
    let mut seen = vec![];
    let mut tested = 0;
    for step in &fx.steps {
        seen.push(observe(&o));
        if tested < max_per_battle && rng.chance(rate) {
            check_boundary(&o, &origin, &history, &seen, pool, rng, c);
            tested += 1;
        }
        let calls: Vec<Call> = step
            .choices
            .iter()
            .map(|ch| (if ch.side == "p1" { 0 } else { 1 }, ch.input.clone(), ch.ok))
            .collect();
        replay(&mut o, &calls, "original");
        history.push(calls);
    }
    assert!(o.state.ended, "fixture did not end");
    c.battles += 1;
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |key: &str| args.iter().position(|s| s == key).and_then(|i| args.get(i + 1)).cloned();
    let jobs: usize = get("--jobs").map_or(3, |s| s.parse().unwrap());
    let rate: f64 = get("--rate").map_or(0.15, |s| s.parse().unwrap());
    let max_per_battle: usize = get("--max-per-battle").map_or(3, |s| s.parse().unwrap());
    let seed: u64 = get("--seed").map_or(0xDE7E_2A11, |s| s.parse().unwrap());
    let mut files = vec![];
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") {
            i += 2;
        } else {
            files.push(args[i].clone());
            i += 1;
        }
    }
    let mut total = Counts::default();
    let start = std::time::Instant::now();
    for path in &files {
        let fixtures: Vec<Fixture> = difftest::runner::open_input(path)
            .unwrap()
            .lines()
            .map(|l| l.unwrap())
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(&l).unwrap())
            .collect();
        // Replacement sets come from other battles of the same corpus.
        let pool: Vec<SetDef> = fixtures
            .iter()
            .take(400)
            .flat_map(|f| f.teams.iter().map(|t| TeamDef::unpack(t).unwrap()))
            .flat_map(|t| t.sets.into_iter().take(t.len as usize))
            .collect();
        let chunk = fixtures.len().div_ceil(jobs.max(1));
        let parts: Vec<Counts> = std::thread::scope(|s| {
            let handles: Vec<_> = fixtures
                .chunks(chunk.max(1))
                .enumerate()
                .map(|(j, part)| {
                    let pool = &pool;
                    s.spawn(move || {
                        let mut c = Counts::default();
                        let mut rng = Rng(seed ^ (j as u64).wrapping_mul(0x9E37_79B9));
                        for fx in part {
                            run(fx, pool, &mut rng, rate, max_per_battle, &mut c);
                        }
                        c
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("check failed")).collect()
        });
        let mut c = Counts::default();
        parts.iter().for_each(|p| c.add(p));
        println!("{path}: {c:?}");
        total.add(&c);
    }
    println!("TOTAL {:?} in {:.1}s", total, start.elapsed().as_secs_f64());
    assert_eq!(total.refused_modified, 0, "never-revealed Pokemon were modified");
    println!("DETERMINIZE CHECK PASSED");
}
