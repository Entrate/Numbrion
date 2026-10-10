//! Determinization reference test (`Battle::replace_hidden_set`, battle/determinize.rs).
//!
//! At random decision boundaries of random-policy battles (constructor.tsv teams, with and
//! without explicit genders), swap a never-revealed opponent set and require:
//! 1. the reference `from_team_defs(s0, resolved teams after the swap)` (s0 = PRNG right after
//!    the original construction), replaying the same choice history, reproduces the original
//!    log, the viewer's requests and the PRNG at every boundary, and ends with the same
//!    `BattleState`, resolved teams and requests as the swapped battle;
//! 2. when the swap keeps the constructor's gender-draw count, construction from the original
//!    seed and the swapped unresolved inputs gives the same state too;
//! 3. after the boundary, the swapped and reference battles play out bit-identically under
//!    identical choices (logs, both requests, PRNG, outcome), with or without a common reseed;
//! 4. every other opponent Pokemon is refused with the right reason and leaves the battle intact.
//!
//! `NUMBRION_DETERMINIZE_BATTLES` scales the battle count (default 120).
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
use std::sync::Arc;

type B = Battle<TextLog>;
/// The choices one side submitted at a boundary.
type Step = (SideId, Vec<SlotChoice>);

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

fn names() -> [String; 2] {
    ["Player 1".into(), "Player 2".into()]
}

fn build(seed: [u16; 4], p1: &Arc<TeamDef>, p2: &Arc<TeamDef>) -> B {
    B::from_team_defs(seed, p1.clone(), p2.clone(), names(), TextLog::default()).unwrap()
}

fn needs_action(b: &B, side: SideId) -> bool {
    matches!(b.side_request_kind(side), RequestKind::Move | RequestKind::Switch)
        && b.legal_actions(side).slot_count > 0
}

/// A random legal joint choice: a switch with probability 0.15 when one is offered, else a
/// uniform move option. Joint constraints are checked by the engine's own predicate.
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

/// Apply random choices for every side that must act; returns them in submission order.
fn advance(b: &mut B, rng: &mut Rng) -> Vec<Step> {
    let acting: Vec<_> = [SideId(0), SideId(1)]
        .into_iter()
        .filter(|&s| needs_action(b, s))
        .collect();
    assert!(!acting.is_empty(), "boundary without an acting side");
    acting
        .into_iter()
        .map(|side| {
            let choice = pick(b, side, rng);
            b.choose_typed(side, &choice).unwrap();
            (side, choice)
        })
        .collect()
}

fn replay(b: &mut B, steps: &[Step]) {
    for (side, choice) in steps {
        b.choose_typed(*side, choice).unwrap();
    }
}

/// What one boundary shows: PRNG, turn, both requests, end.
fn observe(b: &B) -> ([u16; 4], u32, [Option<String>; 2], bool) {
    (b.seed(), b.turn(), [b.request_json(0), b.request_json(1)], b.state.ended)
}

fn draws_gender(set: &SetDef) -> bool {
    set.gender == Gender::Unspecified
        && !matches!(dex::species(set.species).gender, "M" | "F" | "N")
}

fn strip_genders(team: &mut TeamDef) {
    for set in &mut team.sets[..team.len as usize] {
        set.gender = Gender::Unspecified;
    }
}

#[derive(Default, Debug)]
struct Counts {
    boundaries: usize,
    swaps: usize,
    pre_start: usize,
    replacement: usize,
    same_seed: usize,
    draw_count_changed: usize,
    inherited_gender: usize,
    gender_required: usize,
    battle_start: usize,
    refused_revealed: usize,
    /// Never revealed, yet changed since construction: expected zero (see `hidden_mons`).
    refused_modified: usize,
    continued_boundaries: usize,
}

struct Origin {
    seed: [u16; 4],
    /// PRNG right after construction (the constructor's gender draws done).
    s0: [u16; 4],
    raw: [Arc<TeamDef>; 2],
}

/// Swap one hidden opponent set of `o` (at the boundary after `history`) and check the contract.
fn check_boundary(
    o: &B,
    origin: &Origin,
    history: &[Vec<Step>],
    seen: &[([u16; 4], u32, [Option<String>; 2], bool)],
    pool: &[SetDef],
    rng: &mut Rng,
    counts: &mut Counts,
) {
    counts.boundaries += 1;
    let viewer = SideId(rng.below(2) as u8);
    let opp = 1 - viewer.0 as usize;
    let any_set = &pool[rng.below(pool.len())];
    let hidden = o.hidden_mons(viewer);
    // Refusals: own Pokemon, and every opponent Pokemon outside `hidden_mons`.
    let mut probe = o.clone();
    let before = format!("{:?}", probe.state);
    let own = o.state.sides[viewer.0 as usize].party[0];
    assert_eq!(probe.replace_hidden_set(viewer, own, any_set), Err(HiddenSetError::NotOpponent));
    for &m in &o.state.sides[opp].party[..o.state.sides[opp].pokemon_count as usize] {
        if hidden & (1 << m.0) != 0 {
            continue;
        }
        let e = probe.replace_hidden_set(viewer, m, any_set).unwrap_err();
        match e {
            HiddenSetError::Revealed => {
                assert_ne!(o.revealed_mons() & (1 << m.0), 0);
                counts.refused_revealed += 1;
            }
            HiddenSetError::Modified => counts.refused_modified += 1,
            HiddenSetError::BattleStartSpecies => counts.battle_start += 1,
            e => panic!("unexpected refusal {e:?}"),
        }
        assert_eq!(format!("{:?}", probe.state), before, "a refused swap changed the battle");
    }
    let candidates: Vec<MonId> = (0..12)
        .filter(|&i| hidden & (1 << i) != 0)
        .map(MonId)
        .collect();
    if candidates.is_empty() {
        return;
    }
    for &m in &candidates {
        assert_eq!(o.revealed_mons() & (1 << m.0), 0);
        assert_eq!(m.side().0 as usize, opp);
    }
    let mon = candidates[rng.below(candidates.len())];
    let index = mon.0 as usize % 6;
    let mut set = pool[rng.below(pool.len())].clone();
    if rng.chance(0.4) {
        set.gender = Gender::Unspecified;
    }
    let raw_old = &origin.raw[opp].sets[index];
    let mut r = o.clone();
    let mut result = r.replace_hidden_set(viewer, mon, &set);
    if result == Err(HiddenSetError::GenderRequired) {
        assert!(draws_gender(&set) && !draws_gender(raw_old));
        counts.gender_required += 1;
        set.gender = if rng.chance(0.5) { Gender::Male } else { Gender::Female };
        result = r.replace_hidden_set(viewer, mon, &set);
    }
    match result {
        Ok(()) => {}
        Err(HiddenSetError::BattleStartSpecies) => {
            counts.battle_start += 1;
            return;
        }
        Err(e) => panic!("hidden_mons promised a swap, got {e:?}"),
    }
    counts.swaps += 1;
    counts.pre_start += usize::from(!o.state.started);
    counts.replacement += usize::from(
        (0..2).any(|s| o.side_request_kind(SideId(s)) == RequestKind::Switch),
    );
    if draws_gender(&set) {
        assert_eq!(r.teams().sides[opp].sets[index].gender, o.teams().sides[opp].sets[index].gender);
        counts.inherited_gender += 1;
    }
    assert_eq!(r.revealed_mons(), o.revealed_mons());
    assert!(r.log.entries.is_empty());
    assert_eq!(r.seed(), o.seed());
    let mut swapped = o.teams().clone();
    Arc::make_mut(&mut swapped.sides[opp]).sets[index] = r.teams().sides[opp].sets[index].clone();
    assert_eq!(r.teams(), &swapped, "only the swapped set may change");

    // 1. Resolved reference: all genders explicit, so construction draws nothing.
    let t = r.teams().clone();
    let mut f = build(origin.s0, &t.sides[0], &t.sides[1]);
    assert_eq!(f.seed(), origin.s0);
    assert_eq!(f.teams(), r.teams());
    if o.state.started {
        f.start().unwrap();
    }
    for (k, steps) in history.iter().enumerate() {
        let got = observe(&f);
        assert_eq!((got.0, got.1, &got.2[viewer.0 as usize]), (seen[k].0, seen[k].1, &seen[k].2[viewer.0 as usize]),
            "reference diverged before boundary {k}");
        replay(&mut f, steps);
    }
    assert_eq!(f.log.entries, o.log.entries, "reference history log differs");
    assert_eq!(format!("{:?}", f.state), format!("{:?}", r.state), "state differs from reference");
    assert_eq!(observe(&f), observe(&r));

    // 2. Same-seed reference from the swapped unresolved inputs.
    if draws_gender(raw_old) == draws_gender(&set) {
        counts.same_seed += 1;
        let mut raw = origin.raw.clone();
        Arc::make_mut(&mut raw[opp]).sets[index] = set.clone();
        let mut g = build(origin.seed, &raw[0], &raw[1]);
        assert_eq!(g.teams(), r.teams());
        if o.state.started {
            g.start().unwrap();
        }
        for steps in history {
            replay(&mut g, steps);
        }
        assert_eq!(g.log.entries, o.log.entries);
        assert_eq!(format!("{:?}", g.state), format!("{:?}", r.state), "same-seed state differs");
    } else {
        counts.draw_count_changed += 1;
    }

    // 3. Identical future under identical choices, optionally after a common reseed.
    if rng.chance(0.5) {
        let seed = rng.seed();
        r.state.prng = Prng::from_seed(seed);
        f.state.prng = Prng::from_seed(seed);
    }
    if !r.state.started {
        r.start().unwrap();
        f.start().unwrap();
    }
    let offset = f.log.entries.len() - r.log.entries.len();
    let mut play = Rng(rng.next());
    for _ in 0..400 {
        assert_eq!(observe(&r), observe(&f));
        assert_eq!(r.log.entries[..], f.log.entries[offset..]);
        assert_eq!(r.outcome(), f.outcome());
        counts.continued_boundaries += 1;
        if r.state.ended {
            break;
        }
        let steps = advance(&mut r, &mut play);
        replay(&mut f, &steps);
    }
}

fn load() -> (Vec<([u16; 4], TeamDef, TeamDef)>, Vec<SetDef>) {
    let mut battles = vec![];
    let mut pool = vec![];
    for line in include_str!("data/constructor.tsv").lines().filter(|s| s.starts_with("B\t")) {
        let f: Vec<_> = line.split('\t').collect();
        let seed: Vec<u16> = f[1].split(',').map(|n| n.parse().unwrap()).collect();
        let p1 = TeamDef::unpack(f[3]).unwrap();
        let p2 = TeamDef::unpack(f[4]).unwrap();
        // Three one-Pokemon constructor probes cannot play doubles.
        if p1.len < 2 || p2.len < 2 {
            continue;
        }
        for t in [&p1, &p2] {
            pool.extend(t.sets[..t.len as usize].iter().cloned());
        }
        battles.push((seed.try_into().unwrap(), p1, p2));
    }
    (battles, pool)
}

#[test]
fn hidden_set_swap_matches_construction_and_history_replay() {
    let n: usize = std::env::var("NUMBRION_DETERMINIZE_BATTLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let (battles, pool) = load();
    let mut rng = Rng(0xD37E_2141);
    let mut counts = Counts::default();
    for i in 0..n {
        let (seed, mut p1, mut p2) = battles[i % battles.len()].clone();
        // Explicit genders as generated; all unspecified (constructor draws); p2 only.
        match i % 3 {
            1 => {
                strip_genders(&mut p1);
                strip_genders(&mut p2);
            }
            2 => strip_genders(&mut p2),
            _ => {}
        }
        let seed = if i < battles.len() { seed } else { rng.seed() };
        let raw = [Arc::new(p1), Arc::new(p2)];
        let mut o = build(seed, &raw[0], &raw[1]);
        let origin = Origin { seed, s0: o.seed(), raw };
        let mut history: Vec<Vec<Step>> = vec![];
        let mut seen = vec![];
        if rng.chance(0.1) {
            check_boundary(&o, &origin, &history, &seen, &pool, &mut rng, &mut counts);
        }
        o.start().unwrap();
        let mut tested = 0;
        while !o.state.ended && history.len() < 400 {
            seen.push(observe(&o));
            if tested < 3 && rng.chance(0.12) {
                check_boundary(&o, &origin, &history, &seen, &pool, &mut rng, &mut counts);
                tested += 1;
            }
            history.push(advance(&mut o, &mut rng));
        }
    }
    eprintln!("{counts:?}");
    assert!(counts.swaps > n / 2, "too few swaps exercised: {counts:?}");
    assert!(counts.pre_start > 0 && counts.replacement > 0 && counts.same_seed > 0);
    assert!(counts.draw_count_changed > 0 && counts.inherited_gender > 0 && counts.gender_required > 0);
    assert_eq!(counts.refused_modified, 0, "never-revealed Pokemon changed: {counts:?}");
}
