//! Deterministic WorldBuilder availability benchmark.
//!
//! From the repository root:
//! mkdir -p scratch
//! cargo run -p pyengine --release --locked --features search --example world_robustness -- \
//!   --battles 200 --seed 1 --pool crates/pyengine/testdata/pool-400.txt.gz --json scratch/worlds.json
//! --config particles=8,segment_budget=1000,rebuild_budget=16,fresh_per_update=1,shown_target=0.85
//! overrides WorldConfig fields. --max-turns defaults to 40; unfinished games are reported as capped.
//!
//! Both players use the same policy: 6% voluntary switches, 18% status moves when both categories
//! exist, otherwise damaging moves weighted by base power, enemy targets preferred, 8% Tera per
//! eligible move. Viewer sides alternate. Battle/choice/world seeds use separate streams, so changing
//! WorldConfig or the builder implementation cannot change the observed games. Every viewer request,
//! including forced switches and decisions after a failure, calls worlds::<NoLog>(8, seed).
//!
//! Timings include update and the eight returned clones, exclude observation ingestion and request
//! verification. JSON retains every decision and all diagnostics. Ignore keys ending in _ms when
//! checking determinism; transcript_hash also checks identical observations across branches.
//!
//! Collapse means an update with no surviving old particles, including retries after failures.
//! Initial population is reported separately; zero_survivor_updates includes it. The collapses
//! counter identifies recovery even when set repair or backtracking avoids a fresh build. Failure reasons
//! describe last_failure(), the last rejected candidate, rather than every rejection in the update.
//! Rates use Wilson 95% intervals over decisions; decisions within a battle are correlated.

#[cfg(not(feature = "search"))]
fn main() {
    eprintln!("world_robustness requires --features search");
    std::process::exit(2);
}

#[cfg(feature = "search")]
mod bench {
    use _numbrion::pool::TeamPool;
    use engine::{
        Battle, dex,
        ids::SideId,
        log::{NoLog, TextLog},
        search::{
            SplitMix64, protocol,
            sets::SetPool,
            world::{WorldBuilder, WorldConfig, WorldStats},
        },
        state::choices::{ChoiceKind, ChosenMoveKind, RequestKind, SlotChoice},
    };
    use std::{collections::BTreeMap, fmt::Write, path::PathBuf, sync::Arc, time::Instant};

    type B = Battle<TextLog>;
    const WORLDS: usize = 8;
    const BUCKETS: [&str; 4] = ["1-5", "6-10", "11-15", "16+"];

    struct Options {
        battles: usize,
        seed: u64,
        pool: PathBuf,
        json: Option<PathBuf>,
        max_turns: u32,
        config: WorldConfig,
    }

    fn options() -> Result<Option<Options>, String> {
        let mut o = Options {
            battles: 200,
            seed: 1,
            pool: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/pool-400.txt.gz")),
            json: None,
            max_turns: 40,
            config: WorldConfig::default(),
        };
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            if flag == "--help" || flag == "-h" {
                println!(
                    "world_robustness --battles 200 --seed 1 --pool PATH [--json PATH] [--max-turns 40]\n\
                    --config particles=8,segment_budget=1000,rebuild_budget=16,fresh_per_update=1,shown_target=0.85"
                );
                return Ok(None);
            }
            let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
            let invalid = || format!("invalid value for {flag}: {value}");
            match flag.as_str() {
                "--battles" => o.battles = value.parse().map_err(|_| invalid())?,
                "--seed" => o.seed = value.parse().map_err(|_| invalid())?,
                "--pool" => o.pool = value.into(),
                "--json" => o.json = Some(value.into()),
                "--max-turns" => o.max_turns = value.parse().map_err(|_| invalid())?,
                "--config" => {
                    for entry in value.split(',') {
                        let (key, v) = entry.split_once('=').ok_or_else(|| format!("expected key=value: {entry}"))?;
                        let bad = || format!("invalid config: {entry}");
                        match key.trim() {
                            "particles" => o.config.particles = v.trim().parse().map_err(|_| bad())?,
                            "segment_budget" => o.config.segment_budget = v.trim().parse().map_err(|_| bad())?,
                            "rebuild_budget" => o.config.rebuild_budget = v.trim().parse().map_err(|_| bad())?,
                            "fresh_per_update" => o.config.fresh_per_update = v.trim().parse().map_err(|_| bad())?,
                            "shown_target" => o.config.shown_target = v.trim().parse().map_err(|_| bad())?,
                            _ => return Err(format!("unknown config field: {key}")),
                        }
                    }
                }
                _ => return Err(format!("unknown argument: {flag}")),
            }
        }
        if o.battles == 0 || o.max_turns == 0 || o.config.particles == 0 || o.config.segment_budget == 0 {
            return Err("battles, max-turns, particles and segment_budget must be positive".into());
        }
        if !o.config.shown_target.is_finite() || !(0.0..=1.0).contains(&o.config.shown_target) {
            return Err("shown_target must be finite and in [0,1]".into());
        }
        Ok(Some(o))
    }

    fn names() -> [String; 2] {
        ["Alice".into(), "Bob".into()]
    }

    fn needs(b: &B, side: SideId) -> bool {
        !b.state.sides[side.0 as usize].choice.cant_undo
            && matches!(b.side_request_kind(side), RequestKind::Move | RequestKind::Switch)
    }

    #[derive(Clone, Copy)]
    struct Candidate {
        choice: SlotChoice,
        can_tera: bool,
        weight: usize,
    }

    #[derive(Default)]
    struct Slot {
        damage: Vec<Candidate>,
        status: Vec<Candidate>,
        other: Vec<Candidate>,
    }

    fn weighted(list: &[Candidate], rng: &mut SplitMix64) -> Candidate {
        let mut draw = rng.below(list.iter().map(|c| c.weight).sum());
        for &c in list {
            if draw < c.weight {
                return c;
            }
            draw -= c.weight;
        }
        unreachable!()
    }

    // Legal choices, player view and choice text follow tests/world.rs. Only the policy differs.
    fn pick(b: &B, side: SideId, rng: &mut SplitMix64) -> Result<Vec<SlotChoice>, String> {
        let legal = b.legal_actions(side);
        let party = b.state.sides[side.0 as usize].party;
        let mut slots = Vec::new();
        for ls in &legal.slots[..legal.slot_count as usize] {
            let mut slot = Slot::default();
            if ls.automatic_pass {
                slot.other.push(Candidate { choice: SlotChoice::default(), can_tera: false, weight: 1 });
            } else {
                for m in &ls.moves[..ls.move_count as usize] {
                    let damage = m.move_kind == ChosenMoveKind::Dex
                        && dex::move_data(m.move_id).category != dex::Category::Status;
                    let enemy = damage && (1..=2).any(|loc| m.targets.contains(loc));
                    for loc in -2..=2 {
                        if !m.targets.contains(loc) || enemy && loc <= 0 {
                            continue;
                        }
                        let choice = SlotChoice {
                            kind: ChoiceKind::Move,
                            move_slot: m.move_slot,
                            target_loc: loc,
                            move_id: m.move_id,
                            move_kind: m.move_kind,
                            ..SlotChoice::default()
                        };
                        let weight =
                            if damage { dex::move_data(m.move_id).base_power.clamp(40, 140) as usize } else { 1 };
                        let list = if damage { &mut slot.damage } else { &mut slot.status };
                        list.push(Candidate { choice, can_tera: m.can_terastallize, weight });
                    }
                }
                for p in 0..6 {
                    if (ls.switch_mask | ls.revival_mask) & (1 << p) != 0 {
                        slot.other.push(Candidate {
                            choice: SlotChoice {
                                kind: ChoiceKind::Switch,
                                switch_to: party[p],
                                ..SlotChoice::default()
                            },
                            can_tera: false,
                            weight: 1,
                        });
                    }
                }
                if ls.can_pass {
                    slot.other.push(Candidate { choice: SlotChoice::default(), can_tera: false, weight: 1 });
                }
            }
            slots.push(slot);
        }
        for _ in 0..500 {
            let mut tera = false;
            let choice: Vec<_> = slots
                .iter()
                .map(|s| {
                    let list = if s.damage.is_empty() && s.status.is_empty() || !s.other.is_empty() && rng.chance(0.06)
                    {
                        &s.other
                    } else if s.damage.is_empty() || !s.status.is_empty() && rng.chance(0.18) {
                        &s.status
                    } else {
                        &s.damage
                    };
                    let mut c = weighted(list, rng);
                    if !tera && c.can_tera && rng.chance(0.08) {
                        c.choice.tera = true;
                        tera = true;
                    }
                    c.choice
                })
                .collect();
            if b.is_legal_joint_choice(side, &choice) {
                return Ok(choice);
            }
        }
        // Forced replacements can share targets or require passes; exhaust the tiny joint space.
        let options: Vec<Vec<_>> = slots
            .iter()
            .map(|s| s.damage.iter().chain(&s.status).chain(&s.other).map(|c| c.choice).collect())
            .collect();
        for &a in &options[0] {
            if options.len() == 1 && b.is_legal_joint_choice(side, &[a]) {
                return Ok(vec![a]);
            }
            if options.len() == 2 {
                for &c in &options[1] {
                    if b.is_legal_joint_choice(side, &[a, c]) {
                        return Ok(vec![a, c]);
                    }
                }
            }
        }
        Err("no legal joint choice".into())
    }

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
            } else {
                if !entries[i].is_empty() {
                    out.push(entries[i].clone());
                }
                i += 1;
            }
        }
        out
    }

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
                        write!(t, " {:+}", c.target_loc).unwrap();
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

    fn stats_delta(a: WorldStats, b: WorldStats) -> WorldStats {
        let mut delta = WorldStats::default();
        delta.attempts = b.attempts - a.attempts;
        delta.segments = b.segments - a.segments;
        delta.builds = b.builds - a.builds;
        delta.deaths = b.deaths - a.deaths;
        delta.copies = b.copies - a.copies;
        delta.swaps = b.swaps - a.swaps;
        delta.collapses = b.collapses - a.collapses;
        delta.restores = b.restores - a.restores;
        delta.backtracks = b.backtracks - a.backtracks;
        delta.set_repair_attempts = b.set_repair_attempts - a.set_repair_attempts;
        delta.set_repairs = b.set_repairs - a.set_repairs;
        delta.repair_replays = b.repair_replays - a.repair_replays;
        delta.repair_draw_shifts = b.repair_draw_shifts - a.repair_draw_shifts;
        delta
    }

    fn reason(text: &str, lines: &[String]) -> String {
        let underlying = if let Some(rest) = text.split("Budget { line: ").nth(1) {
            let line = rest.split('}').next().and_then(|n| n.trim().parse::<usize>().ok());
            let cmd = line
                .and_then(|i| lines.get(i))
                .and_then(|l| l.strip_prefix('|'))
                .and_then(|l| l.split('|').next())
                .unwrap_or("<end/request>");
            format!("Budget/{}/{cmd}", if text.contains("stuck at line") { "stuck" } else { "budget" })
        } else if text.contains("Swap:") {
            "Swap".into()
        } else if text.contains("Choice:") {
            "Choice".into()
        } else if text.contains("Desync:") || text.contains("inconsistent observation stream") {
            "Desync".into()
        } else if text.is_empty() {
            "<no diagnostic>".into()
        } else {
            "Other".into()
        };
        if text.contains("Illusion") { format!("Illusion/{underlying}") } else { underlying }
    }

    fn hash_text(hash: &mut u64, text: &str) {
        for b in text.bytes().chain([0]) {
            *hash = (*hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
    }

    struct Decision {
        battle: usize,
        turn: u32,
        seed: u64,
        success: bool,
        phase: &'static str,
        wall_ms: f64,
        alive_before: usize,
        alive_after: usize,
        stats: WorldStats,
        choice: String,
        error: Option<String>,
        last_failure: String,
        failure_reason: Option<String>,
    }

    struct Game {
        battle: usize,
        viewer: u8,
        teams: [usize; 2],
        battle_seed: [u16; 4],
        builder_seed: u64,
        turns: u32,
        ended: bool,
        decisions: usize,
        transcript_hash: u64,
    }

    // Small JSON writer avoids adding dependencies to the benchmark or changing Cargo.lock.
    fn quote(s: &str) -> String {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c <= '\u{1f}' => write!(out, "\\u{:04x}", c as u32).unwrap(),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn object(fields: &[(&str, String)]) -> String {
        format!("{{{}}}", fields.iter().map(|(k, v)| format!("{}:{v}", quote(k))).collect::<Vec<_>>().join(","))
    }

    fn array(items: impl IntoIterator<Item = String>) -> String {
        format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
    }

    fn stats_json(s: WorldStats) -> String {
        object(&[
            ("attempts", s.attempts.to_string()),
            ("segments", s.segments.to_string()),
            ("builds", s.builds.to_string()),
            ("deaths", s.deaths.to_string()),
            ("copies", s.copies.to_string()),
            ("swaps", s.swaps.to_string()),
            ("collapses", s.collapses.to_string()),
            ("restores", s.restores.to_string()),
            ("backtracks", s.backtracks.to_string()),
            ("set_repair_attempts", s.set_repair_attempts.to_string()),
            ("set_repairs", s.set_repairs.to_string()),
            ("repair_replays", s.repair_replays.to_string()),
            ("repair_draw_shifts", s.repair_draw_shifts.to_string()),
        ])
    }

    fn rate(n: usize, failed: usize) -> (f64, f64, f64) {
        if n == 0 {
            return (0.0, 0.0, 1.0);
        }
        let n = n as f64;
        let p = failed as f64 / n;
        let z = 1.959963984540054;
        let denom = 1.0 + z * z / n;
        let center = (p + z * z / (2.0 * n)) / denom;
        let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denom;
        (p, (center - half).max(0.0), (center + half).min(1.0))
    }

    fn rate_json(n: usize, failed: usize) -> String {
        let (p, lo, hi) = rate(n, failed);
        object(&[
            ("decisions", n.to_string()),
            ("failures", failed.to_string()),
            ("failure_rate", if n == 0 { "null".into() } else { p.to_string() }),
            ("wilson_95", if n == 0 { "null".into() } else { format!("[{lo},{hi}]") }),
        ])
    }

    fn timing(values: &[f64]) -> String {
        let mut v = values.to_vec();
        v.sort_by(f64::total_cmp);
        if v.is_empty() {
            return object(&[
                ("count", "0".into()),
                ("mean_ms", "null".into()),
                ("p50_ms", "null".into()),
                ("p95_ms", "null".into()),
                ("max_ms", "null".into()),
            ]);
        }
        let percentile = |p: f64| v[(p * v.len() as f64).ceil() as usize - 1];
        object(&[
            ("count", v.len().to_string()),
            ("mean_ms", (v.iter().sum::<f64>() / v.len() as f64).to_string()),
            ("p50_ms", percentile(0.5).to_string()),
            ("p95_ms", percentile(0.95).to_string()),
            ("max_ms", v.last().unwrap().to_string()),
        ])
    }

    fn bucket(turn: u32) -> usize {
        match turn {
            0..=5 => 0,
            6..=10 => 1,
            11..=15 => 2,
            _ => 3,
        }
    }

    fn report(o: &Options, pool: &TeamPool, sets: &SetPool, pool_hash: u64, games: &[Game], ds: &[Decision]) -> String {
        let failures = ds.iter().filter(|d| !d.success).count();
        let collapses = ds.iter().filter(|d| d.phase == "collapse").count();
        let recovered = ds.iter().filter(|d| d.phase == "collapse" && d.success).count();
        let from_particles = ds.iter().filter(|d| d.phase == "collapse" && d.alive_before > 0).count();
        let empty_retries = collapses - from_particles;
        let recovery = WorldStats {
            attempts: ds.iter().map(|d| d.stats.attempts).sum(),
            segments: ds.iter().map(|d| d.stats.segments).sum(),
            builds: ds.iter().map(|d| d.stats.builds).sum(),
            deaths: ds.iter().map(|d| d.stats.deaths).sum(),
            copies: ds.iter().map(|d| d.stats.copies).sum(),
            swaps: ds.iter().map(|d| d.stats.swaps).sum(),
            collapses: ds.iter().map(|d| d.stats.collapses).sum(),
            restores: ds.iter().map(|d| d.stats.restores).sum(),
            backtracks: ds.iter().map(|d| d.stats.backtracks).sum(),
            set_repair_attempts: ds.iter().map(|d| d.stats.set_repair_attempts).sum(),
            set_repairs: ds.iter().map(|d| d.stats.set_repairs).sum(),
            repair_replays: ds.iter().map(|d| d.stats.repair_replays).sum(),
            repair_draw_shifts: ds.iter().map(|d| d.stats.repair_draw_shifts).sum(),
        };
        let mut reasons = BTreeMap::<&str, usize>::new();
        for d in ds {
            if let Some(r) = d.failure_reason.as_deref() {
                *reasons.entry(r).or_default() += 1;
            }
        }
        let buckets = BUCKETS
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let n = ds.iter().filter(|d| bucket(d.turn) == i).count();
                let f = ds.iter().filter(|d| bucket(d.turn) == i && !d.success).count();
                (*label, rate_json(n, f))
            })
            .collect::<Vec<_>>();
        let timings = ["all", "survivors existed", "collapse", "initialization"].map(|phase| {
            let v: Vec<_> = ds.iter().filter(|d| phase == "all" || d.phase == phase).map(|d| d.wall_ms).collect();
            (phase, timing(&v))
        });
        let game_turns: Vec<_> = games.iter().map(|g| g.turns as f64).collect();
        let (p, lo, hi) = rate(ds.len(), failures);
        println!(
            "{} battles, {} decisions; failures {failures} ({:.2}%, Wilson 95% {:.2}-{:.2}%)",
            games.len(),
            ds.len(),
            p * 100.0,
            lo * 100.0,
            hi * 100.0
        );
        println!(
            "collapses {collapses} ({from_particles} from particles, {empty_retries} empty retries), recovered {recovered}; initializations {}; zero-survivor updates {}",
            games.len(),
            ds.iter().filter(|d| d.phase != "survivors existed").count()
        );
        println!(
            "ended {}, capped {}; turns {}",
            games.iter().filter(|g| g.ended).count(),
            games.iter().filter(|g| !g.ended).count(),
            timing(&game_turns).replace("_ms", "")
        );
        println!("recovery: collapses {}, restores {}, backtracks {}, set_repairs {}",
            recovery.collapses, recovery.restores, recovery.backtracks, recovery.set_repairs);
        for (label, counts) in &buckets {
            println!("turns {label}: {counts}");
        }
        for (phase, times) in &timings {
            println!("timing {phase}: {times}");
        }
        println!("failure reasons: {}", object(&reasons.iter().map(|(k, n)| (*k, n.to_string())).collect::<Vec<_>>()));
        object(&[
            ("schema_version", "1".into()),
            ("policy_version", "1".into()),
            ("seed", o.seed.to_string()),
            ("battles", games.len().to_string()),
            ("max_turns", o.max_turns.to_string()),
            ("pool", quote(&o.pool.to_string_lossy())),
            ("pool_teams", pool.len().to_string()),
            ("pool_sets", sets.len().to_string()),
            ("pool_distinct_sets", sets.distinct().to_string()),
            ("pool_hash", quote(&format!("{pool_hash:016x}"))),
            ("worlds_per_decision", WORLDS.to_string()),
            (
                "config",
                object(&[
                    ("particles", o.config.particles.to_string()),
                    ("segment_budget", o.config.segment_budget.to_string()),
                    ("rebuild_budget", o.config.rebuild_budget.to_string()),
                    ("fresh_per_update", o.config.fresh_per_update.to_string()),
                    ("shown_target", o.config.shown_target.to_string()),
                ]),
            ),
            (
                "summary",
                object(&[
                    ("availability", rate_json(ds.len(), failures)),
                    ("turn_buckets", object(&buckets)),
                    ("collapses", collapses.to_string()),
                    ("collapses_from_particles", from_particles.to_string()),
                    ("empty_retries", empty_retries.to_string()),
                    ("recovered", recovered.to_string()),
                    ("restores", recovery.restores.to_string()),
                    ("backtracks", recovery.backtracks.to_string()),
                    ("set_repairs", recovery.set_repairs.to_string()),
                    ("stats", stats_json(recovery)),
                    ("initializations", games.len().to_string()),
                    (
                        "initialization_failures",
                        ds.iter().filter(|d| d.phase == "initialization" && !d.success).count().to_string(),
                    ),
                    ("zero_survivor_updates", ds.iter().filter(|d| d.phase != "survivors existed").count().to_string()),
                    ("failure_reasons", object(&reasons.iter().map(|(k, n)| (*k, n.to_string())).collect::<Vec<_>>())),
                    ("timing", object(&timings)),
                    ("ended", games.iter().filter(|g| g.ended).count().to_string()),
                    ("capped", games.iter().filter(|g| !g.ended).count().to_string()),
                    ("turns", timing(&game_turns).replace("_ms", "")),
                ]),
            ),
            (
                "games",
                array(games.iter().map(|g| {
                    object(&[
                        ("battle", g.battle.to_string()),
                        ("viewer", g.viewer.to_string()),
                        ("teams", array(g.teams.map(|n| n.to_string()))),
                        ("battle_seed", array(g.battle_seed.map(|n| n.to_string()))),
                        ("builder_seed", g.builder_seed.to_string()),
                        ("turns", g.turns.to_string()),
                        ("ended", g.ended.to_string()),
                        ("decisions", g.decisions.to_string()),
                        ("transcript_hash", quote(&format!("{:016x}", g.transcript_hash))),
                    ])
                })),
            ),
            (
                "decisions",
                array(ds.iter().map(|d| {
                    object(&[
                        ("battle", d.battle.to_string()),
                        ("turn", d.turn.to_string()),
                        ("worlds_seed", d.seed.to_string()),
                        ("success", d.success.to_string()),
                        ("phase", quote(d.phase)),
                        ("wall_ms", d.wall_ms.to_string()),
                        ("alive_before", d.alive_before.to_string()),
                        ("alive_after", d.alive_after.to_string()),
                        ("stats_delta", stats_json(d.stats)),
                        ("choice", quote(&d.choice)),
                        ("error", d.error.as_deref().map_or("null".into(), quote)),
                        ("last_failure", quote(&d.last_failure)),
                        ("failure_reason", d.failure_reason.as_deref().map_or("null".into(), quote)),
                    ])
                })),
            ),
        ])
    }

    pub fn run() -> Result<(), String> {
        let Some(o) = options()? else {
            return Ok(());
        };
        let pool = TeamPool::load(&[&o.pool], None).map_err(|e| e.to_string())?;
        let mut sets = SetPool::new();
        let mut pool_hash = 0xcbf29ce484222325;
        for i in 0..pool.len() {
            sets.add_team(pool.get(i)).map_err(|e| format!("pool row {i}: {}", e.0))?;
            hash_text(&mut pool_hash, pool.get(i));
        }
        sets.shrink_to_fit();
        let sets = Arc::new(sets);
        eprintln!(
            "loaded {} teams, {} distinct sets; seed {}, {} battles",
            pool.len(),
            sets.distinct(),
            o.seed,
            o.battles
        );
        let mut master = SplitMix64(o.seed);
        let mut decisions = Vec::new();
        let mut games = Vec::new();
        let start = Instant::now();
        for battle in 0..o.battles {
            let mut setup = master.fork();
            let teams = [setup.below(pool.len()), setup.below(pool.len())];
            let battle_seed = setup.seed();
            let builder_seed = setup.next_u64();
            let mut play_rng = setup.fork();
            let mut world_rng = setup.fork();
            let viewer = SideId((battle % 2) as u8);
            let p1 = pool.parsed(teams[0]).map_err(|e| format!("pool row {}: {e}", teams[0]))?;
            let p2 = pool.parsed(teams[1]).map_err(|e| format!("pool row {}: {e}", teams[1]))?;
            let mut b = B::from_team_defs(battle_seed, p1, p2, names(), TextLog::default()).map_err(|e| e.0)?;
            b.start().map_err(|e| format!("battle {battle} start: {e:?}"))?;
            let own = b.teams().sides[viewer.0 as usize].clone();
            let mut w = WorldBuilder::new(viewer, own, names(), sets.clone(), o.config, builder_seed)
                .map_err(|e| e.to_string())?;
            let mut cursor = 0;
            let mut lines = Vec::new();
            let mut transcript_hash = 0xcbf29ce484222325;
            let first = decisions.len();
            let mut last_turn = 0;
            while !b.state.ended && b.turn() <= o.max_turns {
                let observed = player_view(&b.log.entries, cursor, viewer);
                for l in &observed {
                    hash_text(&mut transcript_hash, l);
                    if protocol::is_compared(l) {
                        lines.push(l.clone());
                    }
                }
                w.push_lines(observed.iter().map(String::as_str));
                cursor = b.log.entries.len();
                let mut acted = false;
                let order = [viewer, SideId(1 - viewer.0)];
                let pending = order.map(|side| needs(&b, side));
                for (side, pending) in order.into_iter().zip(pending) {
                    if !pending {
                        continue;
                    }
                    acted = true;
                    last_turn = b.turn();
                    let choice = pick(&b, side, &mut play_rng)?;
                    let text = choice_text(&b, side, &choice);
                    if side == viewer {
                        let request = b.request_json(viewer.0 as usize).ok_or("viewer has no request")?;
                        hash_text(&mut transcript_hash, &request);
                        w.push_request(&request);
                        let before = w.stats;
                        let alive_before = w.alive();
                        let seed = world_rng.next_u64();
                        let t = Instant::now();
                        let result = w.worlds::<NoLog>(WORLDS, seed);
                        let wall_ms = t.elapsed().as_secs_f64() * 1000.0;
                        let stats = stats_delta(before, w.stats);
                        let success = result.is_ok();
                        let phase = if decisions.len() == first {
                            "initialization"
                        } else if stats.collapses > 0 || alive_before == 0 {
                            "collapse"
                        } else {
                            "survivors existed"
                        };
                        let error = result.as_ref().err().map(ToString::to_string);
                        if let Ok(worlds) = result {
                            if worlds.len() != WORLDS
                                || worlds
                                    .iter()
                                    .any(|x| x.request_json(viewer.0 as usize).as_deref() != Some(request.as_str()))
                            {
                                return Err(format!(
                                    "battle {battle}, turn {}: returned worlds differ from viewer request",
                                    b.turn()
                                ));
                            }
                        }
                        let failure_reason = (!success).then(|| reason(w.last_failure(), &lines));
                        decisions.push(Decision {
                            battle,
                            turn: b.turn(),
                            seed,
                            success,
                            phase,
                            wall_ms,
                            alive_before,
                            alive_after: w.alive(),
                            stats,
                            choice: text.clone(),
                            error,
                            last_failure: w.last_failure().into(),
                            failure_reason,
                        });
                        w.push_choice(&text);
                    }
                    hash_text(&mut transcript_hash, &format!("p{} choice {text}", side.0 + 1));
                    b.choose_typed(side, &choice).map_err(|e| format!("battle {battle}, turn {}: {e}", b.turn()))?;
                }
                if !acted {
                    return Err(format!("battle {battle}, turn {}: no player can act", b.turn()));
                }
            }
            for l in player_view(&b.log.entries, cursor, viewer) {
                hash_text(&mut transcript_hash, &l);
            }
            games.push(Game {
                battle,
                viewer: viewer.0,
                teams,
                battle_seed,
                builder_seed,
                turns: last_turn,
                ended: b.state.ended,
                decisions: decisions.len() - first,
                transcript_hash,
            });
            if (battle + 1) % 10 == 0 || battle + 1 == o.battles {
                eprintln!(
                    "{}/{} battles, {} decisions, {} failures, {:.1}s",
                    battle + 1,
                    o.battles,
                    decisions.len(),
                    decisions.iter().filter(|d| !d.success).count(),
                    start.elapsed().as_secs_f64()
                );
            }
        }
        let json = report(&o, &pool, &sets, pool_hash, &games, &decisions);
        if let Some(path) = &o.json {
            std::fs::write(path, format!("{json}\n")).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(())
    }
}

#[cfg(feature = "search")]
fn main() {
    if let Err(e) = bench::run() {
        eprintln!("world_robustness: {e}");
        std::process::exit(2);
    }
}
