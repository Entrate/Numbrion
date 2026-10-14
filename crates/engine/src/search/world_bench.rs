//! Segment search alone: every segment of random battles is replayed from the true battle state at
//! its start with a reseeded PRNG, so a matching outcome always exists (hidden durations and exact
//! HPs are the true ones; the opponent's choice is still read off the protocol). Measures how often
//! and how fast `advance` finds it. `segment_search_finds_true_outcomes` runs it on the constructor
//! vectors' teams; the ignored benchmark on any team file:
//! `NUMBRION_BENCH_POOL=teams.txt cargo test -p engine --release --features search --lib segment_search_benchmark --
//! --ignored --nocapture` (one packed team per line; `NUMBRION_BENCH_BATTLES`, default 200;
//! `NUMBRION_BENCH_TRACE` prints failures).
use super::*;
use crate::state::choices::ChoiceKind;

fn pick(b: &Battle<DrawLog>, side: SideId, rng: &mut SplitMix64) -> Vec<SlotChoice> {
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
            for (p, &mon) in party.iter().enumerate().take(6) {
                if (ls.switch_mask | ls.revival_mask) & (1 << p) != 0 {
                    other.push(SlotChoice { kind: ChoiceKind::Switch, switch_to: mon, ..SlotChoice::default() });
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
    options.iter().map(|(moves, other)| *moves.iter().chain(other).find(|c| !c.tera).unwrap()).collect()
}

fn choice_text(b: &Battle<DrawLog>, side: SideId, choice: &[SlotChoice]) -> String {
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

/// The viewer's player-view lines of raw entries `from..`.
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

/// Outcome of `run`.
#[derive(Debug, Default)]
struct Report {
    segments: u64,
    reproduced: u64,
    attempts: u64,
    /// Attempts of each reproduced segment.
    successful: Vec<u64>,
    /// Why each other segment failed, and after how many attempts.
    failed: Vec<(String, u64)>,
    /// Reproduced segments that resampled a window of draws (`RESUME`), and those of them whose
    /// record `replay_set` replayed to the identical state.
    windowed: u64,
    replayed: u64,
}

impl core::fmt::Display for Report {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut ok = self.successful.clone();
        ok.sort_unstable();
        let n = ok.len().max(1);
        let kinds = |pat: &str| self.failed.iter().filter(|r| r.0.starts_with(pat)).count();
        write!(
            f,
            "{}/{} segments reproduced ({} budget, {} settled, {} exact-HP, {} other); attempts {} ({:.1}/segment); \
             successful: mean {:.2}, median {}, p90 {}, p99 {}",
            self.reproduced,
            self.segments,
            kinds("Budget"),
            kinds("Settled"),
            kinds("ExactHp"),
            self.failed.len() - kinds("Budget") - kinds("Settled") - kinds("ExactHp"),
            self.attempts,
            self.attempts as f64 / self.segments.max(1) as f64,
            ok.iter().sum::<u64>() as f64 / n as f64,
            ok.get(n / 2).copied().unwrap_or(0),
            ok.get(n * 9 / 10).copied().unwrap_or(0),
            ok.get(n * 99 / 100).copied().unwrap_or(0),
        )?;
        write!(f, "; {} of {} windowed records replayed exactly", self.replayed, self.windowed)
    }
}

/// Changes a true state before its segment is searched (`None`: leave the segment out). Gets the
/// opponent's side and the observed lines from the segment on.
type Perturb<'a> = &'a dyn Fn(&mut Battle<DrawLog>, SideId, &[String]) -> Option<()>;

/// Random battles between `teams` (viewer side alternating), up to turn 20; every segment between
/// two viewer decisions is searched from the true state (changed by `perturb`).
fn run(teams: &[&str], battles: usize, trace: bool, perturb: Option<Perturb<'_>>) -> Report {
    let matchups = (0..battles).map(|battle| {
        let mut rng = SplitMix64(9000 + battle as u64);
        (battle, [rng.below(teams.len()), rng.below(teams.len())])
    });
    run_matchups(teams, matchups, trace, perturb)
}

fn run_matchups(teams: &[&str], matchups: impl IntoIterator<Item = (usize, [usize; 2])>,
    trace: bool, perturb: Option<Perturb<'_>>) -> Report {
    let mut pool = SetPool::new();
    for t in teams {
        pool.add_team(t).unwrap();
    }
    let pool = Arc::new(pool);
    let names = ["Alice".to_string(), "Bob".to_string()];
    let mut report = Report::default();
    for (battle, [i, j]) in matchups {
        let mut rng = SplitMix64(9000 + battle as u64);
        // Keep the battle and choice streams after the two matchup draws, including pinned cases.
        rng.next_u64();
        rng.next_u64();
        let (Ok(a), Ok(b)) = (TeamDef::unpack(teams[i]), TeamDef::unpack(teams[j])) else { continue };
        let Ok(mut real) =
            Battle::from_team_defs(rng.seed(), Arc::new(a), Arc::new(b), names.clone(), DrawLog::default())
        else {
            continue;
        };
        real.start().unwrap();
        let viewer = SideId(battle as u8 % 2);
        let own = real.teams().sides[viewer.0 as usize].clone();
        let config = WorldConfig::default();
        let mut w = WorldBuilder::new(viewer, own, names.clone(), pool.clone(), config, battle as u64).unwrap();
        w.push_lines(player_view(&real.log.text.entries, 0, viewer).iter().map(String::as_str));
        let mut cursor = real.log.text.entries.len();
        // The true state at the last viewer decision, with the observed indices there.
        let mut truth: Option<(Battle<DrawLog>, usize, usize)> = None;
        while !real.state.ended && real.turn() <= 20 {
            for side in [viewer, SideId(1 - viewer.0)] {
                if !needs(&real, side) {
                    continue;
                }
                if side == viewer {
                    w.push_request(&real.request_json(viewer.0 as usize).unwrap());
                    if let Some((mut b, at, choice)) = truth.take() {
                        let mut prng = SplitMix64(rng.next_u64());
                        b.state.prng = crate::prng::Prng::from_seed(prng.seed());
                        if perturb.is_none_or(|f| f(&mut b, SideId(1 - viewer.0), &w.lines[at..]).is_some()) {
                            let repairs = Cell::new(0);
                            let patches = Cell::new(0);
                            let cx = Ctx {
                                viewer,
                                lines: &w.lines,
                                choices: &w.choices,
                                requests: &w.requests,
                                move_orders: &w.move_orders,
                                tracker: &w.tracker,
                                pool: &w.pool,
                                config: &w.config,
                                seen: w.tracker.mons(SideId(1 - viewer.0)),
                                hypothesis: None,
                                repair: true,
                                repairs: &repairs,
                                patches: &patches,
                                rebuild_samples: None,
                            };
                            let mut p = Particle { battle: b, cursor: at, choice, rng: prng.fork(), log_w: 0.0, set_log_w: [0.0; 6],
                                why: String::new(), hypothesis: None, repaired: false, built_at: 0, tera_known: 0, seg: 0,
                                history: Vec::new(), trail: None, death: None, tried: Vec::new() };
                            let mut stats = WorldStats::default();
                            let result = advance(&cx, &mut p, &mut stats);
                            report.segments += 1;
                            report.attempts += stats.attempts;
                            match result {
                                Ok(()) => {
                                    report.reproduced += 1;
                                    report.successful.push(stats.attempts);
                                    // The record replays exactly (the opponent's bench keeps its set).
                                    let hidden = p.history[0].battle.hidden_mons(viewer);
                                    if p.trail.as_ref().is_some_and(|r| r.points.iter().any(|q| q.1 == RESUME))
                                        && let Some(mon) = (0..6).map(|k| MonId(6 * (1 - viewer.0) + k))
                                            .find(|m| hidden & (1 << m.0) != 0)
                                    {
                                        report.windowed += 1;
                                        let set = p.battle.teams().sides[1 - viewer.0 as usize].sets[mon.0 as usize % 6].clone();
                                        let mut replay_stats = WorldStats::default();
                                        if replay_set(&cx, &p, 0, mon, &set, 0.0, &mut replay_stats).is_some_and(|q| {
                                            format!("{:?}", q.battle.state) == format!("{:?}", p.battle.state)
                                        }) {
                                            report.replayed += 1;
                                        }
                                    }
                                }
                                Err(d) => {
                                    let reason = format!("{d:?}: {}", p.why);
                                    if trace {
                                        eprintln!("bench battle {battle} turn {}: {reason}", real.turn());
                                    }
                                    report.failed.push((reason, stats.attempts));
                                }
                            }
                        }
                    }
                    truth = Some((real.clone(), w.lines.len(), w.choices.len()));
                }
                let choice = pick(&real, side, &mut rng);
                if side == viewer {
                    w.push_choice(&choice_text(&real, side, &choice));
                }
                let (opp, logged) = (SideId(1 - viewer.0), real.log.text.entries.len());
                let waiting = side == viewer && needs(&real, opp);
                real.choose_typed(side, &choice).unwrap();
                // The opponent's choice for this boundary was already in (the viewer's choice alone
                // ran the turn): the snapshot is not a state where both sides still choose.
                if waiting && (real.log.text.entries.len() != logged || !needs(&real, opp)) {
                    truth = None;
                }
            }
            let lines = player_view(&real.log.text.entries, cursor, viewer);
            w.push_lines(lines.iter().map(String::as_str));
            cursor = real.log.text.entries.len();
        }
    }
    report
}

/// The constructor vectors' teams that can fill both starting slots in doubles.
fn constructor_teams() -> Vec<&'static str> {
    include_str!("../../tests/data/constructor.tsv")
        .lines()
        .filter_map(|l| l.strip_prefix("B\t"))
        .flat_map(|l| l.split('\t').skip(2).take(2))
        .filter(|t| TeamDef::unpack(t).is_ok_and(|team| team.len >= 2))
        .collect()
}

#[test]
fn constructor_teams_support_doubles() {
    let teams = constructor_teams();
    assert!(!teams.is_empty());
    for packed in teams {
        let team = Arc::new(TeamDef::unpack(packed).unwrap());
        assert!(team.len >= 2, "{packed}");
        let mut b = Battle::from_team_defs([1, 2, 3, 4], team.clone(), team,
            ["Alice".into(), "Bob".into()], DrawLog::default()).unwrap();
        b.start().unwrap();
    }
}

/// Battle 38, turn 8 of the original constructor benchmark: 32 identical damage percentages
/// rejected the true Cinderace HP, although another damage roll reproduces the observed 22/100.
#[test]
fn first_damage_roll_keeps_true_exact_hp() {
    let fixture = |label: &str, side: usize| {
        include_str!("../../tests/data/constructor.tsv").lines()
            .find(|l| l.starts_with("B\t") && l.rsplit('\t').next() == Some(label)).unwrap()
            .split('\t').nth(3 + side).unwrap()
    };
    let teams = [fixture("generated-6", 1), fixture("generated-5", 0)];
    let target = |_: &mut Battle<DrawLog>, _: SideId, lines: &[String]| {
        let at = lines.iter().position(|l| l == "|-damage|p2a: Cinderace|22/100")?;
        assert!(!lines[..at].iter().any(|l| hp_subject(l) == Some((SideId(1), "Cinderace"))));
        Some(())
    };
    let report = run_matchups(&teams, [(38, [0, 1])], true, Some(&target));
    assert_eq!(report.segments, 1, "{report}");
    assert_eq!(report.reproduced, 1, "{report}");
}

/// A matching outcome exists for every segment here; the search must find nearly all of them, with
/// few attempts. The search before resampling windows and the best-replay focus reproduced 94.9%
/// of 7127 such segments with 88 attempts per segment (`segment_search_benchmark`, 600 battles).
#[test]
fn segment_search_finds_true_outcomes() {
    let report = run(&constructor_teams(), 40, false, None);
    eprintln!("{report}");
    assert!(report.segments >= 300, "{report}");
    assert!(report.reproduced * 100 >= report.segments * 97, "{report}");
    assert!(report.attempts <= report.segments * 45, "{report}");
    // Segment records with resampled windows replay to the identical state (`replay_set`).
    assert!(report.windowed >= 50 && report.replayed == report.windowed, "{} of {}", report.replayed, report.windowed);
}

#[test]
#[ignore = "segment search benchmark"]
fn segment_search_benchmark() {
    let text = std::env::var("NUMBRION_BENCH_POOL").map(|p| std::fs::read_to_string(p).unwrap());
    let teams: Vec<&str> = match &text {
        Ok(text) => text.lines().filter(|l| !l.is_empty()).collect(),
        Err(_) => constructor_teams(),
    };
    let battles = std::env::var("NUMBRION_BENCH_BATTLES").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
    let start = std::time::Instant::now();
    let report = run(&teams, battles, std::env::var_os("NUMBRION_BENCH_TRACE").is_some(), None);
    eprintln!("benchmark: {report}; {:.1}s", start.elapsed().as_secs_f64());
}

/// An opponent's exact HP moved to another value its shown percentage allows: where the segment
/// shows that it no longer fits, the world dies as `ExactHp` (a signal for re-running earlier
/// segments).
#[test]
fn exact_hp_dead_ends_are_reported() {
    let perturb = |b: &mut Battle<DrawLog>, opp: SideId, _: &[String]| -> Option<()> {
        let side = &b.state.sides[opp.0 as usize];
        let big = |m: MonId| m != MonId::NONE && b.state.pokemon[m.0 as usize].max_hp > 100;
        let mon = side.active.iter().copied().find(|&m| big(m))?;
        let p = &mut b.state.pokemon[mon.0 as usize];
        let shown = |hp: u16| (100 * hp as u32).div_ceil(p.max_hp as u32);
        let other =
            [p.hp.wrapping_sub(1), p.hp + 1].into_iter().find(|&hp| hp > 0 && hp < p.max_hp && shown(hp) == shown(p.hp))?;
        p.hp = other;
        Some(())
    };
    let report = run(&constructor_teams(), 40, false, Some(&perturb));
    eprintln!("{report}");
    let exact: Vec<_> = report.failed.iter().filter(|f| f.0.starts_with("ExactHp")).collect();
    assert!(exact.len() >= 5, "{report}");
    for (reason, _) in &exact {
        let (line, since) = (field(reason, "line: "), field(reason, "since: "));
        assert!(since <= line, "{reason}");
    }
}

/// The opponent shows a move the world's set cannot choose this turn (here: no PP left): no
/// outcome reproduces it, and the world gives up after a few exploring choices.
#[test]
fn unavailable_moves_end_the_segment_early() {
    let perturb = |b: &mut Battle<DrawLog>, opp: SideId, lines: &[String]| -> Option<()> {
        let shown = lines.iter().take_while(|l| !l.starts_with("|turn|")).find_map(|l| {
            let l = Line::parse(l)?;
            let id = parse_ident(l.arg(0))?;
            let shown = (id.position, dex::lookup(EffectKind::Move, l.arg(1)));
            (l.cmd() == "move" && id.side == opp && !l.has_tag("from")).then_some(shown)
        })?;
        let mon = b.state.sides[opp.0 as usize].active[shown.0? as usize];
        let p = &mut b.state.pokemon[mon.0 as usize];
        let slots = if p.virtual_move_count > 0 {
            &mut p.virtual_move_slots[..p.virtual_move_count as usize]
        } else {
            &mut p.base_move_slots[..p.move_count as usize]
        };
        let slot = slots.iter_mut().find(|m| Some(m.id) == shown.1)?;
        slot.pp = 0;
        Some(())
    };
    let report = run(&constructor_teams(), 30, false, Some(&perturb));
    eprintln!("{report}");
    let unavailable: Vec<_> = report.failed.iter().filter(|f| f.0.contains("cannot choose")).collect();
    assert!(unavailable.len() >= 20, "{report}");
    assert!(unavailable.iter().all(|f| f.1 <= UNAVAILABLE_BUDGET as u64), "{report}");
}

fn field(text: &str, name: &str) -> usize {
    let at = text.find(name).unwrap() + name.len();
    text[at..].split(|c: char| !c.is_ascii_digit()).next().unwrap().parse().unwrap()
}

/// A never-revealed bench Pokemon of a random-gender species got its gender by a coin; when it
/// appears with the other gender, worlds take the shown one instead of dying.
#[test]
fn a_bench_pokemon_appears_with_its_shown_gender() {
    const LATIAS: &str =
        "Latias||SoulDew|Levitate|mistball,recover,dracometeor,tailwind||85,,85,85,85,85|F|,0,,,,||80|,,,,,Steel";
    const MESPRIT: &str =
        "Mesprit||LifeOrb|Levitate|protect,psychic,nastyplot,dazzlinggleam||85,,85,85,85,85|N|,0,,,,||86|,,,,,Psychic";
    const OPP: [&str; 3] = [
        "Umbreon||Leftovers|InnerFocus|snarl,protect,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Dark",
        "Blastoise||WhiteHerb|Torrent|shellsmash,protect,muddywater,dragonpulse||85,,85,85,85,85|F|,0,,,,||83|,,,,,\
         Dragon",
        "Bastiodon||Leftovers|Sturdy|foulplay,wideguard,irondefense,bodypress||85,,85,85,85,85|M|,0,,,,||90|,,,,,\
         Fighting",
    ];
    let mut pool = SetPool::new();
    pool.add_team(&OPP.join("]")).unwrap();
    let pool = Arc::new(pool);
    let names = ["Alice".to_string(), "Bob".to_string()];
    let (viewer, opp) = (SideId(0), SideId(1));
    let mut deaths = 0;
    for seed in 0..6 {
        let own = Arc::new(TeamDef::unpack(&[LATIAS, MESPRIT].join("]")).unwrap());
        let theirs = Arc::new(TeamDef::unpack(&OPP.join("]")).unwrap());
        let mut real = Battle::from_team_defs([1, 2, 3, seed], own, theirs, names.clone(), DrawLog::default()).unwrap();
        real.start().unwrap();
        let own = real.teams().sides[0].clone();
        let mut w =
            WorldBuilder::new(viewer, own, names.clone(), pool.clone(), WorldConfig::default(), seed as u64).unwrap();
        w.push_lines(player_view(&real.log.text.entries, 0, viewer).iter().map(String::as_str));
        let cursor = real.log.text.entries.len();
        w.push_request(&real.request_json(0).unwrap());
        w.update().unwrap();
        // Turn 1: the opponent switches Umbreon out for Bastiodon.
        let choice = pick(&real, viewer, &mut SplitMix64(seed as u64));
        w.push_choice(&choice_text(&real, viewer, &choice));
        real.choose_typed(viewer, &choice).unwrap();
        let legal = real.legal_actions(opp);
        let m = legal.slots[1].moves[0];
        let target = (-2..=2).find(|&l| m.targets.contains(l)).unwrap();
        let theirs = [
            SlotChoice { kind: ChoiceKind::Switch, switch_to: real.state.sides[1].party[2], ..SlotChoice::default() },
            SlotChoice {
                kind: ChoiceKind::Move,
                move_slot: m.move_slot,
                target_loc: target,
                move_id: m.move_id,
                move_kind: m.move_kind,
                ..SlotChoice::default()
            },
        ];
        real.choose_typed(opp, &theirs).unwrap();
        let lines = player_view(&real.log.text.entries, cursor, viewer);
        assert!(lines.iter().any(|l| l.starts_with("|switch|p2a: Bastiodon|Bastiodon, L90, M|")), "{lines:?}");
        w.push_lines(lines.iter().map(String::as_str));
        if needs(&real, viewer) {
            w.push_request(&real.request_json(0).unwrap());
        }
        let before = w.stats.deaths;
        w.update().unwrap();
        deaths += w.stats.deaths - before;
    }
    assert_eq!(deaths, 0);
}
