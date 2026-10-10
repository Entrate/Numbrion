use engine::{
    Battle,
    dex::{self, *},
    ids::*,
    state::*,
    teams::TeamDef,
};
fn numbers<T: std::str::FromStr>(s: &str) -> Vec<T>
where
    T::Err: std::fmt::Debug,
{
    s.split(',').map(|n| n.parse().unwrap()).collect()
}
#[test]
fn constructor_matches_pinned_showdown() {
    let mut battle: Option<Battle> = None;
    let mut label = "";
    let mut count = 0;
    let mut changed_seeds = 0;
    for line in include_str!("data/constructor.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let f: Vec<_> = line.split('\t').collect();
        if f[0] == "B" {
            label = f[5];
            let seed: [u16; 4] = numbers(f[1]).try_into().unwrap();
            let b = Battle::new(seed, f[3], f[4]).unwrap_or_else(|e| panic!("{label}: {e}"));
            let after: [u16; 4] = numbers(f[2]).try_into().unwrap();
            assert_eq!(b.state.prng.seed(), after, "{label}: PRNG");
            changed_seeds += usize::from(seed != after);
            assert_eq!(b.state.effect_order, 0);
            assert_eq!(b.state.turn, 0);
            assert!(!b.state.started);
            assert_eq!(b.state.phase, Phase::Created);
            assert_eq!(b.state.field.pseudo_weather.len, 1);
            let rule = b.state.field.pseudo_weather.cells[0];
            assert_eq!(
                b.state.effects.cells[rule.0 as usize].id,
                RULE_SLEEPCLAUSEMOD
            );
            assert_eq!(b.state.effects.cells[rule.0 as usize].target, Holder::NONE);
            assert_eq!(b.state.sides[0].active, [MonId::NONE; 2]);
            assert_eq!(b.state.sides[1].active, [MonId::NONE; 2]);
            for (si, side) in b.state.sides.iter().enumerate() {
                assert_eq!(side.pokemon_left, side.pokemon_count);
                for pos in 0..side.pokemon_count as usize {
                    assert_eq!(side.party[pos], MonId((si * 6 + pos) as u8));
                }
            }
            battle = Some(b);
            count += 1;
        } else {
            assert_eq!(f[0], "P");
            let b = battle.as_ref().unwrap();
            let mon = MonId(f[1].parse().unwrap());
            let p = &b.state.pokemon[mon.0 as usize];
            let set = &b.teams().sides[mon.side().0 as usize].sets[mon.0 as usize % 6];
            assert_eq!(set.gender.protocol(), f[2], "{label} {mon:?} gender");
            assert_eq!(set.name, f[3], "{label} name");
            assert_eq!(b.details(mon), f[4], "{label} details");
            assert_eq!(p.hp, f[5].parse::<u16>().unwrap(), "{label} {mon:?} hp");
            assert_eq!(p.max_hp, p.hp);
            assert_eq!(
                p.stored_stats,
                numbers::<u16>(f[6]).as_slice(),
                "{label} {mon:?} stats"
            );
            assert_eq!(p.base_stored_stats, p.stored_stats);
            assert_eq!(dex::effect(p.species).key, f[7]);
            assert_eq!(p.base_species, p.species);
            assert_eq!(
                if p.ability == EffectId::NONE {
                    ""
                } else {
                    dex::effect(p.ability).key
                },
                f[8]
            );
            assert_eq!(p.base_ability, p.ability);
            assert_eq!(
                if p.item == EffectId::NONE {
                    ""
                } else {
                    dex::effect(p.item).key
                },
                f[9]
            );
            let types: Vec<_> = f[10].split(',').map(|t| dex::type_id(t).unwrap()).collect();
            assert_eq!(&p.types[..types.len()], types);
            assert_eq!(p.apparent_types, p.types);
            assert_eq!(p.weighthg, f[11].parse::<u16>().unwrap());
            assert_eq!(p.speed, f[12].parse::<u16>().unwrap());
            let moves: Vec<_> = f[13].split(',').collect();
            assert_eq!(p.move_slots().len(), moves.len());
            for (m, expected) in p.move_slots().iter().zip(moves) {
                let expected: Vec<_> = expected.split(':').collect();
                assert_eq!(dex::effect(m.id).key, expected[0]);
                assert_eq!(m.pp, expected[1].parse::<u8>().unwrap());
                assert_eq!(m.max_pp, expected[2].parse::<u8>().unwrap());
                assert_eq!(m.target, move_data(m.id).target);
                assert_eq!(m.flags, 0);
                assert_eq!(m.disabled_source, EffectId::NONE);
            }
            assert_eq!(set.tera_type, dex::type_id(f[14]).unwrap());
            assert_eq!(set.level, f[15].parse::<u16>().unwrap());
            assert_eq!(set.evs, numbers::<u8>(f[16]).as_slice(), "{label} EVs");
            assert_eq!(set.ivs, numbers::<u8>(f[17]).as_slice(), "{label} IVs");
            assert_eq!(p.status, Status::None);
            assert_eq!(p.flags, mon_flags::KNOWN_TYPE | mon_flags::NEWLY_SWITCHED);
            assert_eq!(p.volatiles.len, 0);
            for (cell, id, target) in [
                (p.species_state, p.species, Holder::NONE),
                (p.status_state, EffectId::NONE, Holder::NONE),
                (p.ability_state, p.ability, Holder::mon(mon)),
                (p.item_state, p.item, Holder::mon(mon)),
            ] {
                let c = &b.state.effects.cells[cell.0 as usize];
                assert_eq!(c.id, id);
                assert_eq!(c.effect_order, 0);
                assert_eq!(c.target, target);
                assert_eq!(c.duration, -1);
            }
        }
    }
    assert_eq!(count, 323);
    assert!(
        changed_seeds >= 20,
        "Missing-gender fixtures must exercise constructor PRNG draws"
    );
}
#[test]
fn dex_ids_and_scope_agree() {
    for line in include_str!("data/scope-counts.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let (k, list) = line.split_once('\t').unwrap();
        let kind = match k {
            "species" => EffectKind::Species,
            "moves" => EffectKind::Move,
            "abilities" => EffectKind::Ability,
            "items" => EffectKind::Item,
            "conditions" => EffectKind::Condition,
            "rules" => EffectKind::Rule,
            _ => panic!("kind"),
        };
        let keys: Vec<_> = list.split(',').collect();
        let data = dex::table(kind);
        assert_eq!(data.len(), keys.len());
        for (i, (entry, key)) in data.iter().zip(keys).enumerate() {
            assert_eq!(entry.key, key);
            assert_eq!(entry.id.0, data[0].id.0 + i as u16);
            assert_eq!(entry.id.kind(), Some(kind));
            assert_eq!(dex::lookup(kind, key), Some(entry.id));
        }
    }
    assert_eq!(MANIFESTS.len(), EFFECT_COUNT);
    assert_ne!(MOVE_PROTECT, CONDITION_PROTECT);
    assert_eq!(SPECIES_COUNT, 572);
    assert_eq!(MOVE_COUNT, 359);
    assert_eq!(ABILITY_COUNT, 207);
    assert_eq!(ITEM_COUNT, 61);
    assert_eq!(CONDITION_COUNT, 75);
    assert_eq!(RULE_COUNT, 12);
    for h in HOOKS {
        assert!(MANIFESTS[h.effect.0 as usize].has_event(h.event));
    }
    let pressure = MANIFESTS[ABILITY_PRESSURE.0 as usize].hooks();
    assert!(pressure.iter().any(|h| h.event == EventId::DeductPP));
    assert_eq!(
        canonical_effect(CONDITION_SLEEPCLAUSEMOD),
        RULE_SLEEPCLAUSEMOD
    );
    assert!(
        SPECIES_CONDITION_VIEWS
            .iter()
            .any(|v| v.species == SPECIES_ZACIAN)
    );
    assert!(
        !SPECIES_CONDITION_VIEWS
            .iter()
            .any(|v| v.species == SPECIES_ZACIANCROWNED)
    );
    assert_eq!(
        move_data(MOVE_SHELLSMASH)
            .effects
            .boosts
            .iter()
            .map(|b| b.stat)
            .collect::<Vec<_>>(),
        [
            StatId::Def,
            StatId::SpD,
            StatId::Atk,
            StatId::SpA,
            StatId::Spe
        ]
    );
    assert_eq!(
        IMMUNITY_CHART[type_id("Fire").unwrap().0 as usize - 1][ImmunityId::Brn as usize],
        3
    );
    let armor = MANIFESTS[ABILITY_BULLETPROOF.0 as usize].hooks();
    assert!(armor.iter().any(|h| h.event == EventId::TryHit));
    let shell = MANIFESTS[ABILITY_SHELLARMOR.0 as usize].hooks();
    assert!(
        shell
            .iter()
            .any(|h| h.value == HookValue::Constant(DataValue::Bool(false)))
    );
    assert!(
        MANIFESTS[CONDITION_AURORAVEIL.0 as usize]
            .hooks()
            .iter()
            .any(|h| h.key == "onSideResidual"
                && h.order == 26.0
                && h.sub_order == 10.0
                && h.value == HookValue::Absent)
    );
}
#[test]
fn codegen_is_deterministic_and_checked_in() {
    use std::{fs, path::PathBuf, process::Command};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let oracle = std::env::var_os("NUMBRION_SHOWDOWN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .expect("Set NUMBRION_SHOWDOWN to the pinned, built Pokemon Showdown checkout");
            PathBuf::from(home).join("src/pokemon-showdown")
        });
    let out = std::env::temp_dir().join(format!("numbrion-dex-test-{}", std::process::id()));
    fs::create_dir_all(&out).unwrap();
    let generate = || {
        let output = Command::new("node")
            .arg(root.join("tools/codegen/gen-dex.mjs"))
            .arg(&oracle)
            .arg(&out)
            .output()
            .expect("Node 24 and pinned built Showdown are required for codegen verification");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let names = [
        "ids_generated.rs",
        "events_generated.rs",
        "hooks_generated.rs",
        "data_generated.rs",
    ];
    generate();
    let first: Vec<_> = names
        .iter()
        .map(|n| fs::read(out.join(n)).unwrap())
        .collect();
    generate();
    for (n, bytes) in names.iter().zip(first) {
        assert_eq!(fs::read(out.join(n)).unwrap(), bytes, "{n} second run");
        assert_eq!(
            fs::read(root.join("crates/engine/src/dex").join(n)).unwrap(),
            bytes,
            "{n} checked-in data"
        );
    }
    fs::remove_dir_all(out).unwrap();
}
#[test]
fn capacities_and_snapshot_size() {
    fn require_copy<T: Copy>() {}
    require_copy::<BattleState>();
    println!(
        "BattleState={} Pokemon={} EffectCell={} Action={} Request={} Arena={}",
        size_of::<BattleState>(),
        size_of::<Pokemon>(),
        size_of::<EffectCell>(),
        size_of::<choices::Action>(),
        size_of::<choices::SideRequest>(),
        size_of::<EffectArena>()
    );
    assert!(TeamDef::unpack("bad").is_err());
}

#[test]
fn removed_effects_keep_captured_objects_until_dispatch_finishes() {
    let mut arena = EffectArena::default();
    let owner = Holder::mon(MonId(0));
    let old = arena.alloc(owner, owner, CONDITION_CONFUSION, 7);
    arena.cells[old.0 as usize].payload.words[0] = 3;
    let captured = arena.pin(old);
    arena.release(old);
    // runEvent may still call captured onBeforeMove. fieldEvent skips the retired object.
    assert!(!arena.is_live(captured));
    assert_eq!(arena.cells[captured.cell.0 as usize].payload.words[0], 3);
    let replacement = arena.alloc(owner, owner, CONDITION_CONFUSION, 8);
    assert_ne!(replacement, old);
    arena.cells[old.0 as usize].clear();
    assert_eq!(arena.cells[old.0 as usize].target, owner);
    arena.unpin(captured);
    let recycled = arena.alloc(owner, owner, CONDITION_CONFUSION, 9);
    assert_eq!(recycled, old);
    assert_ne!(arena.capture(recycled).generation, captured.generation);
    let mut list = EffectList::<3>::default();
    list.push(replacement);
    list.push(recycled);
    let third = arena.alloc(owner, owner, CONDITION_TAUNT, 10);
    list.push(third);
    assert_eq!(list.remove(1), recycled);
    list.push(recycled);
    assert_eq!(list.as_slice(), [replacement, third, recycled]);
}
#[test]
fn compact_attack_history_matches_full_list_readers() {
    #[derive(Clone, Copy)]
    struct Entry {
        source: MonId,
        slot: SlotId,
        damage: Option<u32>,
        this_turn: bool,
    }
    let target = MonId(0);
    let mut state = BattleState::empty([3, 4, 5, 6]);
    let mut full = Vec::<Entry>::new();
    for p in &mut state.pokemon {
        p.flags |= mon_flags::ACTIVE;
    }
    for step in 0..5000 {
        let source = MonId((state.prng.random(11) + 1) as u8);
        let draw = state.prng.random(4);
        let damage = if draw == 0 {
            None
        } else {
            Some(state.prng.random(10))
        };
        state.pokemon[source.0 as usize].position = state.prng.random(6) as u8;
        let slot = SlotId::new(source.side(), state.pokemon[source.0 as usize].position);
        state.record_attack(target, source, MOVE_AVALANCHE, damage);
        full.push(Entry {
            source,
            slot,
            damage,
            this_turn: true,
        });
        if step % 37 == 0 {
            let inactive = MonId((state.prng.random(11) + 1) as u8);
            state.pokemon[inactive.0 as usize].flags &= !mon_flags::ACTIVE;
            state.prune_attacks(target);
            full.retain(|e| state.pokemon[e.source.0 as usize].flags & mon_flags::ACTIVE != 0);
            for e in &mut full {
                e.this_turn = false
            }
            state.seq_at_last_end_turn = state.attack_seq;
            state.pokemon[inactive.0 as usize].flags |= mon_flags::ACTIVE;
        }
        let last = full.last();
        let compact = state.last_attacked_by(target);
        assert_eq!(
            compact.map(|(s, r)| (
                s,
                r.last_slot,
                r.last_damage,
                r.last_seq > state.seq_at_last_end_turn
            )),
            last.map(|e| (e.source, e.slot, e.damage.unwrap_or(0), e.this_turn))
        );
        for filter in [None, Some(false), Some(true)] {
            let last = full.iter().rev().find(|e| {
                e.damage.is_some() && (filter.is_none() || e.source.side() != target.side())
            });
            let compact = state.last_damaged_by(target, filter);
            assert_eq!(
                compact.map(|(s, r)| (
                    s,
                    r.numeric_slot,
                    r.numeric_damage,
                    r.numeric_seq > state.seq_at_last_end_turn
                )),
                last.map(|e| (e.source, e.slot, e.damage.unwrap(), e.this_turn))
            );
        }
        for source in 1..12 {
            assert_eq!(
                state.pokemon[0].attacks[source].flags & 4 != 0,
                full.iter().any(|e| e.source == MonId(source as u8)
                    && e.this_turn
                    && e.damage.is_some_and(|d| d > 0))
            );
        }
    }
}
