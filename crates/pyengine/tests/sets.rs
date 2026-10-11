//! `engine::search::sets::SetPool` on the generated test pool (testdata/pool-400.txt.gz): sizes,
//! shown species against real battle details, sampling frequencies against the pool's empirical
//! ones, and that sampled teams build and start valid battles.
#![cfg(feature = "search")]

use engine::{
    Battle,
    ids::{EffectId, EffectKind},
    log::{NoLog, TextLog},
    search::{
        SplitMix64,
        sets::{SetConstraints, SetPool},
    },
    teams::{Gender, SetDef, TeamDef},
};
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader},
    sync::Arc,
};

fn pool_lines() -> Vec<String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/pool-400.txt.gz");
    let file = std::fs::File::open(path).expect("test pool");
    BufReader::new(flate2::read::MultiGzDecoder::new(file))
        .lines()
        .map(|l| l.unwrap().trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

fn load() -> (SetPool, Vec<TeamDef>) {
    let lines = pool_lines();
    let mut pool = SetPool::new();
    for l in &lines {
        pool.add_team(l).expect("pool team parses");
    }
    (
        pool,
        lines.iter().map(|l| TeamDef::unpack(l).unwrap()).collect(),
    )
}

#[test]
fn sizes() {
    let (pool, teams) = load();
    assert_eq!(teams.len(), 400);
    assert_eq!(pool.len(), 2400);
    // Order-sensitive distinct count (gender blanked), for comparison with the stored count.
    let exact: HashSet<String> = pool_lines()
        .iter()
        .flat_map(|l| {
            l.split(']')
                .map(|r| {
                    let mut f: Vec<&str> = r.split('|').collect();
                    f[7] = "";
                    f.join("|")
                })
                .collect::<Vec<_>>()
        })
        .collect();
    eprintln!(
        "pool-400: {} sets, {} distinct ({:.3}); {} distinct counting move order",
        pool.len(),
        pool.distinct(),
        pool.distinct() as f64 / pool.len() as f64,
        exact.len()
    );
    assert!(pool.distinct() <= exact.len() && exact.len() <= pool.len());
    assert!(pool.distinct() > 0);
}

/// `team` with its sets rotated left by `k`.
fn rotated(team: &TeamDef, k: usize) -> TeamDef {
    let mut t = team.clone();
    t.sets[..t.len as usize].rotate_left(k);
    t
}

/// Every pool set leads a battle once (teams rotated by 0, 2, 4); its `|switch|` line at start
/// must print the species text `shown_species` gives (Terapagos, for example, switches in as
/// "Terapagos" and only then becomes Terapagos-Terastal).
#[test]
fn shown_species_matches_switch_lines() {
    let (pool, teams) = load();
    let illusion = engine::dex::lookup(EffectKind::Ability, "Illusion").unwrap();
    let mut checked = 0;
    for pair in teams.chunks(2) {
        for k in [0, 2, 4] {
            let sides = [rotated(&pair[0], k), rotated(&pair[1], k)];
            let mut b = Battle::from_team_defs(
                [1, 2, 3, 4],
                Arc::new(sides[0].clone()),
                Arc::new(sides[1].clone()),
                ["A".into(), "B".into()],
                TextLog::default(),
            )
            .unwrap();
            b.start().unwrap();
            // Split triples repeat each switch line (secret and public view).
            let mut seen = HashSet::new();
            for line in b.log.entries.iter().filter(|l| l.starts_with("|switch|")) {
                let f: Vec<&str> = line.split('|').collect();
                let (side, slot) = (f[2].as_bytes()[1] - b'1', f[2].as_bytes()[2] - b'a');
                let set = &sides[side as usize].sets[slot as usize];
                let shown = SetPool::shown_species(set);
                assert!(pool.contains_species(shown));
                if seen.insert((side, slot)) {
                    checked += 1;
                }
                if set.ability == illusion {
                    continue; // Zoroark shows the last party member
                }
                assert_eq!(f[3].split(", ").next().unwrap(), shown, "{line}");
            }
        }
    }
    assert_eq!(checked, 2400);
}

#[test]
fn sampling_matches_empirical_frequencies() {
    let (pool, teams) = load();
    let sets: Vec<&SetDef> = teams
        .iter()
        .flat_map(|t| t.sets[..t.len as usize].iter())
        .collect();
    let mut by_species: HashMap<&str, Vec<&SetDef>> = HashMap::new();
    for s in &sets {
        by_species
            .entry(SetPool::shown_species(s))
            .or_default()
            .push(s);
    }
    // The most common species (ties broken by name, for a stable choice).
    let (&species, group) = by_species
        .iter()
        .max_by_key(|(k, v)| (v.len(), std::cmp::Reverse(**k)))
        .unwrap();
    let mut empirical: HashMap<EffectId, f64> = HashMap::new();
    for s in group {
        *empirical.entry(s.item).or_default() += 1.0 / group.len() as f64;
    }
    let n = 40_000;
    let mut sampled: HashMap<EffectId, f64> = HashMap::new();
    let mut rng = SplitMix64(2024);
    for _ in 0..n {
        let s = pool
            .sample(&SetConstraints::new("X", species), &mut rng)
            .unwrap();
        assert!(s.consistent);
        *sampled.entry(s.set.item).or_default() += 1.0 / n as f64;
    }
    eprintln!(
        "{species} ({} sets): empirical {empirical:?} sampled {sampled:?}",
        group.len()
    );
    assert!(sampled.keys().all(|k| empirical.contains_key(k)));
    for (item, &p) in &empirical {
        let q = sampled.get(item).copied().unwrap_or(0.0);
        // 40k draws: standard error below 0.0025.
        assert!(
            (p - q).abs() < 0.015,
            "{species} {item:?}: empirical {p:.3}, sampled {q:.3}"
        );
    }

    // Unrevealed slots: species frequencies over the whole pool.
    let mut empirical: HashMap<&str, f64> = HashMap::new();
    for s in &sets {
        *empirical.entry(SetPool::shown_species(s)).or_default() += 1.0 / sets.len() as f64;
    }
    let mut sampled: HashMap<&str, f64> = HashMap::new();
    for _ in 0..n {
        let s = pool.sample_unrevealed(&[], &mut rng).unwrap();
        *sampled.entry(SetPool::shown_species(&s)).or_default() += 1.0 / n as f64;
    }
    let tv: f64 = empirical
        .iter()
        .map(|(k, p)| (p - sampled.get(k).copied().unwrap_or(0.0)).abs())
        .sum::<f64>()
        / 2.0;
    eprintln!("unrevealed species total variation: {tv:.3}");
    assert!(tv < 0.12, "total variation {tv}");
}

/// The sets of one sampled team: slot 0 constrained by a revealed pool set, the rest unrevealed.
fn sample_team(pool: &SetPool, revealed: &SetDef, rng: &mut SplitMix64) -> TeamDef {
    let c = SetConstraints {
        level: Some(revealed.level),
        gender: revealed.gender,
        moves: &revealed.moves[..(revealed.move_count as usize).min(2)], // Ditto has one move
        item: Some(revealed.item),
        ..SetConstraints::new("Revealed", SetPool::shown_species(revealed))
    };
    let first = pool
        .sample(&c, rng)
        .expect("revealed species is in the pool");
    assert!(first.consistent);
    assert_eq!(first.set.name, "Revealed");
    let mut team = TeamDef::default();
    team.sets[0] = first.set;
    team.len = 1;
    for i in 1..6 {
        let exclude: Vec<EffectId> = team.sets[..i].iter().map(|s| s.species).collect();
        team.sets[i] = pool
            .sample_unrevealed(&exclude, rng)
            .expect("pool has six species");
        team.len += 1;
    }
    team
}

#[test]
fn sampled_teams_build_valid_battles() {
    let (pool, teams) = load();
    let mut rng = SplitMix64(77);
    for (k, t) in teams.iter().enumerate().take(200) {
        let revealed = &t.sets[k % t.len as usize];
        let a = sample_team(&pool, revealed, &mut rng);
        let b = sample_team(&pool, &teams[(k + 1) % teams.len()].sets[0], &mut rng);
        for team in [&a, &b] {
            team.validate().unwrap();
            let bases: HashSet<&str> = team
                .sets
                .iter()
                .map(|s| engine::dex::species(s.species).base_species_name)
                .collect();
            assert_eq!(bases.len(), 6, "species clause");
            assert!(team.sets.iter().all(|s| s.gender != Gender::Unspecified));
        }
        let mut battle = Battle::from_team_defs(
            rng.seed(),
            Arc::new(a.clone()),
            Arc::new(b),
            ["A".into(), "B".into()],
            NoLog,
        )
        .unwrap();
        battle.start().unwrap();
        // The revealed slot shows what was observed.
        assert_eq!(
            SetPool::shown_species(&a.sets[0]),
            SetPool::shown_species(revealed)
        );
        assert_eq!(a.sets[0].level, revealed.level);
        if revealed.gender == Gender::Male || revealed.gender == Gender::Female {
            assert_eq!(a.sets[0].gender, revealed.gender);
        }
    }
}

#[test]
fn deterministic() {
    let (pool, teams) = load();
    let run = || {
        let mut rng = SplitMix64(5);
        (0..50)
            .map(|k| sample_team(&pool, &teams[k].sets[1], &mut rng))
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
