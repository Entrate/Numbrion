//! Fair-information worlds (`engine::search::world`, docs/design/SEARCH-PREREQUISITES.md item 1).
//!
//! * `worlds_reproduce_the_viewer_state`: random battles observed by one player; worlds built from
//!   that player's lines, requests and choices alone must sit at the same boundary with the
//!   viewer's request identical to the real one.
//! * `identical_public_information_gives_identical_worlds` (the leak test): two real battles whose
//!   opponent teams differ only in what the viewer never saw (never-revealed bench sets, which may
//!   be BattleStart species; unused moves, items, abilities and Tera types of revealed Pokemon),
//!   played with the same choices and seed. When the viewer's observations are identical, worlds
//!   and their simulated futures must be identical.
//!
//! `NUMBRION_TEST_BATTLES` scales both (defaults 40 and 60).
#![cfg(feature = "search")]
mod common;

use _numbrion::pool::TeamPool;
use engine::{
    Battle,
    ids::SideId,
    log::TextLog,
    search::{
        SplitMix64,
        sets::SetPool,
        world::{WorldBuilder, WorldConfig},
    },
    state::choices::{ChoiceKind, RequestKind, SlotChoice},
    teams::{SetDef, TeamDef},
};
use std::sync::Arc;
use std::time::Instant;

type B = Battle<TextLog>;

fn set_pool(pool: &TeamPool) -> Arc<SetPool> {
    let mut sets = SetPool::new();
    for i in 0..pool.len() {
        sets.add_team(pool.get(i)).unwrap();
    }
    Arc::new(sets)
}

fn names() -> [String; 2] {
    ["Alice".into(), "Bob".into()]
}

fn needs(b: &B, side: SideId) -> bool {
    !b.state.sides[side.0 as usize].choice.cant_undo
        && matches!(b.side_request_kind(side), RequestKind::Move | RequestKind::Switch)
}

/// A random legal joint choice (switch with probability 0.15 when offered, Tera rarely).
fn pick(b: &B, side: SideId, rng: &mut SplitMix64) -> Vec<SlotChoice> {
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
                    other.push(SlotChoice { kind: ChoiceKind::Switch, switch_to: party[p], ..SlotChoice::default() });
                }
            }
            if ls.can_pass {
                other.push(SlotChoice::default());
            }
            (moves, other)
        })
        .collect();
    for _ in 0..500 {
        let choice: Vec<_> = options
            .iter()
            .map(|(moves, other)| {
                let list = if moves.is_empty() || (!other.is_empty() && rng.chance(0.15)) { other } else { moves };
                list[rng.below(list.len())]
            })
            .collect();
        if choice.iter().filter(|c| c.tera).count() <= 1
            && (choice.iter().all(|c| !c.tera) || rng.chance(0.3))
            && b.is_legal_joint_choice(side, &choice)
        {
            return choice;
        }
    }
    let choice: Vec<_> = options
        .iter()
        .map(|(moves, other)| {
            let list = if moves.is_empty() { other } else { moves };
            *list.iter().find(|c| !c.tera).unwrap_or(&list[0])
        })
        .collect();
    assert!(b.is_legal_joint_choice(side, &choice), "no legal joint choice");
    choice
}

/// The viewer's player view of raw entries `from..` (`extractChannelMessages`).
fn player_view(entries: &[String], from: usize, viewer: SideId) -> Vec<String> {
    let mut out = vec![];
    let mut i = from;
    while i < entries.len() {
        if let Some(owner) = entries[i].strip_prefix("|split|p").and_then(|n| n.parse::<u8>().ok())
            && i + 2 < entries.len()
        {
            let l = if owner == viewer.0 + 1 { &entries[i + 1] } else { &entries[i + 2] };
            if !l.is_empty() {
                out.push(l.clone());
            }
            i += 3;
            continue;
        }
        if !entries[i].is_empty() {
            out.push(entries[i].clone());
        }
        i += 1;
    }
    out
}

/// Showdown choice text of a typed choice (the player's own record of what it sent).
fn choice_text(b: &B, side: SideId, choice: &[SlotChoice]) -> String {
    let party = &b.state.sides[side.0 as usize].party;
    choice
        .iter()
        .map(|c| match c.kind {
            ChoiceKind::Pass => "pass".to_string(),
            ChoiceKind::Switch | ChoiceKind::Revival => {
                format!("switch {}", party.iter().position(|&m| m == c.switch_to).unwrap() + 1)
            }
            ChoiceKind::Move if c.move_slot == 255 => "move 1".to_string(),
            ChoiceKind::Move => {
                let mut t = format!("move {}", c.move_slot + 1);
                if c.target_loc != 0 {
                    t.push_str(&format!(" {:+}", c.target_loc));
                }
                if c.tera {
                    t.push_str(" terastallize");
                }
                t
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// What one player observes, in order.
#[derive(Clone, Debug, PartialEq)]
enum Event {
    Lines(Vec<String>),
    Request(String),
    Choice(String),
}

/// Both sides' choices at one boundary, in submission order (the viewer first).
type Boundary = Vec<(SideId, Vec<SlotChoice>)>;

/// Play (or replay) a battle. With `history`, its choices are applied instead of random ones and
/// `None` is returned if one is rejected.
fn play(
    teams: [Arc<TeamDef>; 2],
    seed: [u16; 4],
    viewer: SideId,
    turns: u32,
    rng: &mut SplitMix64,
    history: Option<&[Boundary]>,
) -> Option<(B, Vec<Event>, Vec<Boundary>)> {
    let [p1, p2] = teams;
    let mut b = B::from_team_defs(seed, p1, p2, names(), TextLog::default()).ok()?;
    b.start().ok()?;
    let mut events = vec![Event::Lines(player_view(&b.log.entries, 0, viewer))];
    let mut cursor = b.log.entries.len();
    let mut played = vec![];
    while !b.state.ended && b.turn() <= turns && history.is_none_or(|h| played.len() < h.len()) {
        let mut boundary = vec![];
        let order = [viewer, SideId(1 - viewer.0)];
        for side in order {
            if !needs(&b, side) {
                continue;
            }
            let choice = match history {
                Some(h) => h[played.len()].iter().find(|(s, _)| *s == side)?.1.clone(),
                None => pick(&b, side, rng),
            };
            if side == viewer {
                events.push(Event::Request(b.request_json(viewer.0 as usize).unwrap()));
                events.push(Event::Choice(choice_text(&b, side, &choice)));
            }
            b.choose_typed(side, &choice).ok()?;
            boundary.push((side, choice));
        }
        if let Some(h) = history
            && h[played.len()].len() != boundary.len()
        {
            return None;
        }
        played.push(boundary);
        events.push(Event::Lines(player_view(&b.log.entries, cursor, viewer)));
        cursor = b.log.entries.len();
    }
    Some((b, events, played))
}

fn builder(viewer: SideId, b: &B, sets: &Arc<SetPool>, events: &[Event], seed: u64) -> WorldBuilder {
    let own = Arc::new((*b.teams().sides[viewer.0 as usize]).clone());
    let mut w = WorldBuilder::new(viewer, own, names(), sets.clone(), WorldConfig::default(), seed).unwrap();
    for e in events {
        match e {
            Event::Lines(l) => w.push_lines(l.iter().map(String::as_str)),
            Event::Request(r) => w.push_request(r),
            Event::Choice(c) => w.push_choice(c),
        }
    }
    w
}

fn teams_of(pool: &TeamPool, i: usize) -> Arc<TeamDef> {
    pool.parsed(i).unwrap()
}

#[test]
fn worlds_reproduce_the_viewer_state() {
    let pool = common::test_pool();
    let sets = set_pool(&pool);
    let mut rng = SplitMix64(7);
    let (mut ok, mut failed, mut checked) = (0, 0, 0);
    let mut stats = engine::search::world::WorldStats::default();
    let n = common::n_battles(40);
    for battle in 0..n {
        let (i, j) = (rng.below(pool.len()), rng.below(pool.len()));
        let viewer = SideId(battle as u8 % 2);
        let turns = 1 + rng.below(8) as u32;
        let Some((b, events, _)) =
            play([teams_of(&pool, i), teams_of(&pool, j)], rng.seed(), viewer, turns, &mut rng, None)
        else {
            continue;
        };
        if b.state.ended || !needs(&b, viewer) {
            continue;
        }
        let mut w = builder(viewer, &b, &sets, &events, rng.next_u64());
        match w.worlds::<TextLog>(4, rng.next_u64()) {
            Ok(worlds) => {
                ok += 1;
                for world in &worlds {
                    checked += 1;
                    assert_eq!(world.turn(), b.turn());
                    assert!(needs(world, viewer));
                    assert_eq!(world.request_json(viewer.0 as usize), b.request_json(viewer.0 as usize));
                    assert_eq!(world.teams().sides[viewer.0 as usize], b.teams().sides[viewer.0 as usize]);
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!("battle {battle}: turn {}: {e}: {}", b.turn(), w.last_failure());
                if std::env::var("NUMBRION_WORLD_DUMP").is_ok_and(|v| v == battle.to_string()) {
                    for e in &events {
                        eprintln!("  {e:?}");
                    }
                    eprintln!("  real opponent: {}", b.teams().sides[1 - viewer.0 as usize].pack());
                }
            }
        }
        let s = w.stats;
        stats.attempts += s.attempts;
        stats.segments += s.segments;
        stats.builds += s.builds;
        stats.deaths += s.deaths;
        stats.copies += s.copies;
        stats.swaps += s.swaps;
    }
    eprintln!("worlds: {ok} positions ok, {failed} failed, {checked} worlds checked; {stats:?}");
    assert!(ok > 0);
    assert!(failed * 5 <= ok + failed, "too many positions without a consistent world: {failed} of {}", ok + failed);
}

/// The opponent team with every detail the viewer never saw changed.
fn disguise(real: &TeamDef, b: &B, opp: SideId, played: &[Boundary], sets: &[SetDef], rng: &mut SplitMix64) -> TeamDef {
    let mut t = real.clone();
    let revealed = b.revealed_mons();
    // Move slots each opponent Pokemon was ever told to use (stable MonIds).
    let mut used = [[false; 4]; 6];
    for boundary in played {
        for (side, choice) in boundary {
            if *side != opp {
                continue;
            }
            for c in choice {
                if c.kind == ChoiceKind::Move && c.move_slot < 4 {
                    // Keep every move id the opponent chose, on whichever Pokemon knows it.
                    for (m, set) in real.sets[..real.len as usize].iter().enumerate() {
                        if let Some(s) = set.moves.iter().position(|&id| id == c.move_id) {
                            used[m][s] = true;
                        }
                    }
                }
            }
        }
    }
    // One kind of change per variant, so most variants stay invisible to the viewer.
    let kind = rng.below(5);
    for m in 0..t.len as usize {
        let mon = opp.0 as usize * 6 + m;
        let other = &sets[rng.below(sets.len())];
        if revealed & (1 << mon) == 0 {
            // Pool sets carry explicit genders, so construction draws the same as before.
            if kind == 0 || rng.chance(0.5) {
                t.sets[m] = other.clone();
            }
            continue;
        }
        if !rng.chance(0.5) {
            continue;
        }
        let set = &mut t.sets[m];
        match kind {
            1 => {
                for slot in 0..set.move_count as usize {
                    if !used[m][slot] {
                        let candidate = other.moves[rng.below(other.move_count.max(1) as usize)];
                        if candidate != engine::ids::EffectId::NONE && !set.moves.contains(&candidate) {
                            set.moves[slot] = candidate;
                        }
                    }
                }
            }
            2 => set.item = other.item,
            3 => set.ability = other.ability,
            _ => set.tera_type = other.tera_type,
        }
    }
    t
}

#[test]
fn identical_public_information_gives_identical_worlds() {
    let pool = common::test_pool();
    let sets = set_pool(&pool);
    let all_sets: Vec<SetDef> = (0..pool.len())
        .flat_map(|i| {
            let t = teams_of(&pool, i);
            t.sets[..t.len as usize].to_vec()
        })
        .collect();
    let mut rng = SplitMix64(11);
    let (mut pairs, mut differed, mut discarded, mut failed) = (0, 0, 0, 0);
    let n = common::n_battles(60);
    for battle in 0..n {
        let (i, j) = (rng.below(pool.len()), rng.below(pool.len()));
        let viewer = SideId(battle as u8 % 2);
        let opp = SideId(1 - viewer.0);
        let seed = rng.seed();
        let turns = 1 + rng.below(6) as u32;
        let teams = [teams_of(&pool, i), teams_of(&pool, j)];
        let Some((a, events_a, played)) = play(teams.clone(), seed, viewer, turns, &mut rng, None) else {
            continue;
        };
        if a.state.ended || !needs(&a, viewer) {
            continue;
        }
        // Try a few disguises; keep the first the viewer cannot tell apart from the real team.
        let mut found = None;
        for _ in 0..12 {
            let hidden = disguise(&teams[opp.0 as usize], &a, opp, &played, &all_sets, &mut rng);
            if hidden.validate().is_err() || hidden == *teams[opp.0 as usize] {
                continue;
            }
            let mut alt = teams.clone();
            alt[opp.0 as usize] = Arc::new(hidden);
            if let Some((b, events_b, _)) = play(alt, seed, viewer, u32::MAX, &mut rng, Some(&played))
                && events_a == events_b
                && needs(&b, viewer)
            {
                found = Some((b, events_b));
                break;
            }
        }
        let Some((b, events_b)) = found else {
            discarded += 1;
            continue;
        };
        if a.teams().sides[opp.0 as usize] != b.teams().sides[opp.0 as usize] {
            differed += 1;
        }
        pairs += 1;
        let world_seed = rng.next_u64();
        let pick_seed = rng.next_u64();
        let mut wa = builder(viewer, &a, &sets, &events_a, world_seed);
        let mut wb = builder(viewer, &b, &sets, &events_b, world_seed);
        let (ra, rb) = (wa.worlds::<TextLog>(4, pick_seed), wb.worlds::<TextLog>(4, pick_seed));
        assert_eq!(ra.is_ok(), rb.is_ok(), "battle {battle}: one observer got worlds, the other did not");
        assert_eq!(wa.stats, wb.stats, "battle {battle}: builder statistics differ");
        assert_eq!(wa.opponent_teams(), wb.opponent_teams(), "battle {battle}: sampled teams differ");
        let (Ok(xs), Ok(ys)) = (ra, rb) else {
            failed += 1;
            continue;
        };
        for (mut x, mut y) in xs.into_iter().zip(ys) {
            assert_eq!(format!("{:?}", x.state), format!("{:?}", y.state), "battle {battle}: world states differ");
            assert_eq!(x.teams(), y.teams());
            // The same search from both: identical simulated futures under identical choices.
            let mut sim = SplitMix64(rng.next_u64());
            for _ in 0..3 {
                if x.state.ended {
                    break;
                }
                for side in [SideId(0), SideId(1)] {
                    if needs(&x, side) {
                        let c = pick(&x, side, &mut sim);
                        x.choose_typed(side, &c).unwrap();
                        y.choose_typed(side, &c).unwrap();
                    }
                }
                assert_eq!(x.log.entries, y.log.entries, "battle {battle}: futures differ");
            }
        }
    }
    eprintln!(
        "leak test: {pairs} pairs ({differed} with different hidden opponent info), {discarded} discarded, {failed} without worlds"
    );
    assert!(differed >= n / 6, "too few informative pairs: {differed}");
}

/// Worlds requested at every decision of a battle, as a self-play actor would: each must reproduce the
/// viewer's current request (pushed before asking), including after Transform and late pushes.
#[test]
fn worlds_follow_a_battle_decision_by_decision() {
    let pool = common::test_pool();
    let sets = set_pool(&pool);
    let mut rng = SplitMix64(23);
    let (mut ok, mut failed) = (0usize, 0usize);
    let n = common::n_battles(12);
    for battle in 0..n {
        let (i, j) = (rng.below(pool.len()), rng.below(pool.len()));
        let viewer = SideId(battle as u8 % 2);
        let Ok(mut b) =
            B::from_team_defs(rng.seed(), teams_of(&pool, i), teams_of(&pool, j), names(), TextLog::default())
        else {
            continue;
        };
        b.start().unwrap();
        let own = Arc::new((*b.teams().sides[viewer.0 as usize]).clone());
        let mut w =
            WorldBuilder::new(viewer, own, names(), sets.clone(), WorldConfig::default(), rng.next_u64()).unwrap();
        w.push_lines(player_view(&b.log.entries, 0, viewer).iter().map(String::as_str));
        let mut cursor = b.log.entries.len();
        let mut broken = false;
        while !b.state.ended && b.turn() <= 10 {
            for side in [viewer, SideId(1 - viewer.0)] {
                if !needs(&b, side) {
                    continue;
                }
                let choice = pick(&b, side, &mut rng);
                if side == viewer {
                    w.push_request(&b.request_json(viewer.0 as usize).unwrap());
                    if !broken {
                        match w.worlds::<TextLog>(2, rng.next_u64()) {
                            Ok(worlds) => {
                                ok += 1;
                                for x in &worlds {
                                    assert_eq!(x.request_json(viewer.0 as usize), b.request_json(viewer.0 as usize));
                                }
                            }
                            Err(e) => {
                                failed += 1;
                                broken = true;
                                eprintln!("battle {battle} turn {}: {e}: {}", b.turn(), w.last_failure());
                            }
                        }
                    }
                    w.push_choice(&choice_text(&b, side, &choice));
                }
                b.choose_typed(side, &choice).unwrap();
            }
            w.push_lines(player_view(&b.log.entries, cursor, viewer).iter().map(String::as_str));
            cursor = b.log.entries.len();
        }
    }
    eprintln!("incremental: {ok} decisions with worlds, {failed} battles lost their worlds");
    assert!(ok > 0 && failed * 3 <= n, "{failed} of {n} battles lost their worlds");
}

/// Fixed simulation seeds, independent of the builder's draws and successes. Also usable on the
/// base implementation: `cargo test ... --test world world_availability_and_timing -- --ignored --nocapture`.
#[test]
#[ignore = "availability and maintenance timing measurement"]
fn world_availability_and_timing() {
    measure_worlds(false);
    measure_worlds(true);
}

fn measure_worlds(illusion: bool) -> (usize, usize) {
    let pool = common::test_pool();
    let sets = set_pool(&pool);
    let candidates: Vec<_> = (0..pool.len())
        .filter(|&i| {
            let t = teams_of(&pool, i);
            t.sets[..t.len as usize].iter().any(|s| s.ability == engine::dex::ABILITY_ILLUSION) == illusion
        })
        .collect();
    let mut times = Vec::new();
    let mut counts = [[0usize; 2]; 2];
    for battle in 0..common::n_battles(12) {
        let mut rng = SplitMix64(2000 + battle as u64);
        let own = teams_of(&pool, rng.below(pool.len()));
        let mut opp = (*teams_of(&pool, candidates[battle % candidates.len()])).clone();
        if illusion {
            let z =
                opp.sets[..opp.len as usize].iter().position(|s| s.ability == engine::dex::ABILITY_ILLUSION).unwrap();
            opp.sets.swap(0, z);
        }
        let mut b = B::from_team_defs(rng.seed(), own, Arc::new(opp), names(), TextLog::default()).unwrap();
        b.start().unwrap();
        let viewer = SideId(0);
        let mut w = WorldBuilder::new(
            viewer,
            b.teams().sides[0].clone(),
            names(),
            sets.clone(),
            WorldConfig::default(),
            3000 + battle as u64,
        )
        .unwrap();
        w.push_lines(player_view(&b.log.entries, 0, viewer).iter().map(String::as_str));
        let mut cursor = b.log.entries.len();
        let mut revealed = false;
        while !b.state.ended && b.turn() <= 10 {
            for side in [viewer, SideId(1)] {
                if !needs(&b, side) {
                    continue;
                }
                let choice = pick(&b, side, &mut rng);
                if side == viewer {
                    w.push_request(&b.request_json(0).unwrap());
                    let start = Instant::now();
                    let result = w.worlds::<TextLog>(2, 4000 + counts.iter().flatten().sum::<usize>() as u64);
                    if std::env::var_os("NUMBRION_WORLD_TRACE").is_some()
                        && let Err(e) = &result
                    {
                        eprintln!(
                            "measurement battle {battle} turn {} revealed {revealed}: {e}; {}",
                            b.turn(),
                            w.last_failure()
                        );
                    }
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                    counts[revealed as usize][result.is_ok() as usize] += 1;
                    if let Ok(worlds) = result {
                        for world in worlds {
                            assert_eq!(world.request_json(0), b.request_json(0));
                        }
                    }
                    w.push_choice(&choice_text(&b, side, &choice));
                }
                b.choose_typed(side, &choice).unwrap();
            }
            let lines = player_view(&b.log.entries, cursor, viewer);
            revealed |= lines.iter().any(|s| s.starts_with("|replace|p2"));
            w.push_lines(lines.iter().map(String::as_str));
            cursor = b.log.entries.len();
        }
    }
    times.sort_by(f64::total_cmp);
    let n = times.len();
    eprintln!(
        "{}: before replace {}/{}; after replace {}/{}; update/worlds ms median {:.3}, mean {:.3}, p90 {:.3}",
        if illusion { "Illusion" } else { "normal" },
        counts[0][1],
        counts[0].iter().sum::<usize>(),
        counts[1][1],
        counts[1].iter().sum::<usize>(),
        times[n / 2],
        times.iter().sum::<f64>() / n as f64,
        times[n * 9 / 10]
    );
    (counts.iter().map(|c| c[1]).sum(), n)
}

#[test]
fn illusion_worlds_follow_random_battles() {
    let (ok, total) = measure_worlds(true);
    assert!(ok * 10 >= total * 9, "Illusion worlds at only {ok}/{total} decisions");
}

fn illusion_fixture() -> (Arc<TeamDef>, Arc<TeamDef>, Arc<SetPool>) {
    let pool = common::test_pool();
    let z = teams_of(&pool, 5).sets[4].clone();
    let b = (0..pool.len())
        .flat_map(|i| teams_of(&pool, i).sets[..6].to_vec())
        .find(|s| s.species == engine::dex::SPECIES_BASTIODON)
        .unwrap();
    let u = TeamDef::unpack(
        "Umbreon||Leftovers|InnerFocus|snarl,protect,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Dark",
    )
    .unwrap()
    .sets[0]
        .clone();
    let l = TeamDef::unpack(
        "Latias||SoulDew|Levitate|mistball,recover,dracometeor,tailwind||85,,85,85,85,85|F|,0,,,,||80|,,,,,Steel",
    )
    .unwrap()
    .sets[0]
        .clone();
    let bl = teams_of(&pool, 5).sets[5].clone();
    let m = TeamDef::unpack(
        "Mesprit||LifeOrb|Levitate|protect,psychic,nastyplot,dazzlinggleam||85,,85,85,85,85|N|,0,,,,||86|,,,,,Psychic",
    )
    .unwrap()
    .sets[0]
        .clone();
    let mut opp = TeamDef { len: 6, ..TeamDef::default() };
    opp.sets = [z, l, u, m, bl, b];
    let mut own = opp.clone();
    own.sets.swap(0, 4);
    own.sets.swap(1, 2);
    let mut sets = SetPool::new();
    for s in &opp.sets {
        sets.add_set(s);
    }
    (Arc::new(own), Arc::new(opp), Arc::new(sets))
}

#[test]
fn bastiodon_uses_zoroark_moves_and_damage_before_replace() {
    let (own, opp, sets) = illusion_fixture();
    let mut b = B::from_team_defs([1, 2, 3, 4], own, opp, names(), TextLog::default()).unwrap();
    b.start().unwrap();
    let mut w =
        WorldBuilder::new(SideId(0), b.teams().sides[0].clone(), names(), sets, WorldConfig::default(), 41).unwrap();
    w.push_lines(player_view(&b.log.entries, 0, SideId(0)).iter().map(String::as_str));
    let mut cursor = b.log.entries.len();
    for turn in 1..=5 {
        w.push_request(&b.request_json(0).unwrap());
        let worlds = w.worlds::<TextLog>(2, turn).unwrap_or_else(|e| panic!("turn {turn}: {e}; {}", w.last_failure()));
        for world in worlds {
            assert_eq!(world.request_json(0), b.request_json(0));
        }
        if turn == 5 {
            break;
        }
        let own = if turn == 4 { "move 4 +1, move 4" } else { "move 2, move 4" };
        let opp = match turn {
            1 => "move 2, move 2",
            3 => "move 4 +2, move 2",
            _ => "move 3, move 2",
        };
        w.push_choice(own);
        b.choose(0, own).unwrap();
        b.choose(1, opp).unwrap();
        let lines = player_view(&b.log.entries, cursor, SideId(0));
        if turn == 3 {
            assert!(lines.iter().any(|s| s.starts_with("|move|p2a: Bastiodon|Sludge Bomb")));
            assert!(lines.iter().any(|s| s.starts_with("|-damage|p2a: Bastiodon|") && s.contains("Life Orb")));
            assert!(!b.log.entries.iter().any(|s| s.starts_with("|replace|")));
        }
        if turn == 4 {
            assert!(lines.iter().any(|s| s.starts_with("|replace|p2a: Zoroark|")));
        }
        w.push_lines(lines.iter().map(String::as_str));
        cursor = b.log.entries.len();
    }
}

#[test]
fn hidden_illusion_and_a_real_disguise_have_identical_world_inputs() {
    let (own, opp, sets) = illusion_fixture();
    let mut alt = (*opp).clone();
    alt.sets.swap(0, 5);
    alt.sets[0].moves[1] = engine::dex::MOVE_PROTECT;
    let mut battles: Vec<_> = [opp, Arc::new(alt)]
        .into_iter()
        .map(|opp| {
            let mut b = B::from_team_defs([1, 2, 3, 4], own.clone(), opp, names(), TextLog::default()).unwrap();
            b.start().unwrap();
            b.choose(0, "move 1, move 4").unwrap();
            b.choose(1, "move 2, move 2").unwrap();
            b
        })
        .collect();
    let a = battles.remove(0);
    let b = battles.remove(0);
    let la = player_view(&a.log.entries, 0, SideId(0));
    let lb = player_view(&b.log.entries, 0, SideId(0));
    assert_eq!(la, lb);
    let config = WorldConfig { particles: 32, rebuild_budget: 64, ..WorldConfig::default() };
    let mut wa = WorldBuilder::new(SideId(0), a.teams().sides[0].clone(), names(), sets.clone(), config, 32).unwrap();
    let mut wb = WorldBuilder::new(SideId(0), b.teams().sides[0].clone(), names(), sets, config, 32).unwrap();
    // Update before and after the same public choice, so the hypothesis prior also runs.
    for w in [&mut wa, &mut wb] {
        let initial = la.iter().position(|s| s == "|turn|1").unwrap() + 1;
        w.push_lines(la[..initial].iter().map(String::as_str));
        w.update().unwrap();
        w.push_choice("move 1, move 4");
        w.push_lines(la[initial..].iter().map(String::as_str));
        w.push_request(&a.request_json(0).unwrap());
        w.update().unwrap();
    }
    assert!(
        wa.opponent_teams().iter().any(|t| { t.sets[..2].iter().any(|s| s.ability == engine::dex::ABILITY_ILLUSION) })
    );
    assert_eq!(wa.stats, wb.stats);
    assert_eq!(wa.opponent_teams(), wb.opponent_teams());
    for (x, y) in wa.worlds::<TextLog>(4, 77).unwrap().into_iter().zip(wb.worlds::<TextLog>(4, 77).unwrap()) {
        assert_eq!(format!("{:?}", x.state), format!("{:?}", y.state));
    }
}
