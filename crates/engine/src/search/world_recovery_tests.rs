fn fixture(extra: impl FnOnce(&mut SetPool, &TeamDef)) -> (WorldBuilder, Battle<DrawLog>) {
    let own = Arc::new(TeamDef::unpack(&[BL, M].join("]")).unwrap());
    let opp = Arc::new(TeamDef::unpack(&[L, U, B].join("]")).unwrap());
    let mut pool = SetPool::new();
    for set in &opp.sets[..3] { pool.add_set(set); }
    extra(&mut pool, &opp);
    let mut real = Battle::from_team_defs([1, 2, 3, 4], own.clone(), opp, ["A".into(), "B".into()],
        DrawLog::default()).unwrap();
    real.start().unwrap();
    let config = WorldConfig { particles: 1, rebuild_budget: 1, fresh_per_update: 0, ..WorldConfig::default() };
    let mut builder = WorldBuilder::new(SideId(0), own, ["A".into(), "B".into()], Arc::new(pool), config, 5).unwrap();
    push(&mut builder, &real);
    builder.update().unwrap();
    (builder, real)
}

fn push(builder: &mut WorldBuilder, real: &Battle<DrawLog>) {
    let mut produced = Vec::new();
    view(&real.log, SideId(0), &mut produced);
    builder.push_lines(produced.iter().map(|&(raw, _)| real.log.text.entries[raw].as_str()));
    builder.push_request(&real.request_json(0).unwrap());
}

fn turn(builder: &mut WorldBuilder, real: &mut Battle<DrawLog>, opponent: &str) {
    builder.push_choice("move 2, move 1");
    real.log.reset();
    real.choose(0, "move 2, move 1").unwrap();
    real.choose(1, opponent).unwrap();
    push(builder, real);
}

fn cx(builder: &WorldBuilder) -> Ctx<'_> {
    Ctx { lines: &builder.lines, choices: &builder.choices, requests: &builder.requests,
        repair: true, ..context(&builder.tracker, &builder.pool, &builder.config) }
}

#[test]
fn repair_retries_use_record_identity_after_constraints_change() {
    let (mut builder, _) = fixture(|pool, team| {
        for _ in 0..300 { pool.add_set(&team.sets[0]); }
        let mut alternate = team.sets[0].clone();
        alternate.moves[0] = dex::MOVE_PSYCHIC;
        for _ in 0..3 { pool.add_set(&alternate); }
        alternate.item = dex::ITEM_CHOICESCARF;
        pool.add_set(&alternate);
    });
    let mut dead = builder.particles[0].clone();
    assert!(!dead.battle.teams().sides[1].sets[0].moves.contains(&dex::MOVE_PSYCHIC));
    builder.push_lines(["|move|p2a: Latias|Psychic|p1a: Blastoise"]);
    dead.death = Some(Dead::Budget { line: dead.cursor });
    let (mut source, mut cached, mut stats) = (None, Vec::new(), WorldStats::default());
    assert!(repair_world(&cx(&builder), &mut dead, &mut 1, 100, &mut stats, &mut source, &mut cached).is_none());
    assert_eq!(dead.tried.len(), 1);
    let first = dead.tried[0];
    builder.push_lines(["|-item|p2a: Latias|Choice Scarf"]);
    assert!(repair_world(&cx(&builder), &mut dead, &mut 1, 100, &mut stats, &mut source, &mut cached).is_none());
    assert_eq!(dead.tried.len(), 2, "the first candidate of the new filter has never been tried");
    assert_ne!(dead.tried[1], first);
    assert_eq!(stats.set_repair_attempts, 2);
    assert!(repair_world(&cx(&builder), &mut dead, &mut 1, 100, &mut stats, &mut source, &mut cached).is_none());
    assert_eq!(stats.set_repair_attempts, 2, "an already tried record stays excluded");
}

#[test]
fn entry_of_a_preloaded_set_pins_its_snapshot() {
    let (mut builder, mut real) = fixture(|_, _| {});
    let anchor = builder.particles[0].history.last().unwrap().clone();
    assert_eq!(set_name(&builder.particles[0].battle, MonId(8)), "Bastiodon");
    let swaps = builder.stats.swaps;
    turn(&mut builder, &mut real, "switch 3, move 2");
    builder.update().unwrap();
    assert_eq!(builder.stats.swaps, swaps, "the entry used the set already in the team");
    assert!(anchor.introduces.load(Ordering::Relaxed));
    let p = &mut builder.particles[0];
    p.seg = 14;
    remember(p, Arc::new(Snapshot::of(p)));
    assert!(p.history.iter().any(|s| Arc::ptr_eq(s, &anchor)), "entry anchors survive thinning");
}

#[test]
fn newest_repair_anchor_owns_the_new_set_weight_and_entry_flag() {
    let (mut builder, mut real) = fixture(|_, _| {});
    let dead = builder.particles[0].clone();
    let index = dead.history.len() - 1;
    let old = dead.history[index].clone();
    let mut set = dead.battle.teams().sides[1].sets[2].clone();
    set.tera_type = dex::type_id("Water").unwrap();
    let mut stats = WorldStats::default();
    let mut repaired = replay_set(&cx(&builder), &dead, index, MonId(8), &set, -1.5, &mut stats).unwrap();
    let new = repaired.history.last().unwrap().clone();
    assert!(!Arc::ptr_eq(&new, &old));
    assert_eq!(new.battle.teams().sides[1].sets[2], set);
    assert_eq!(new.log_w, old.log_w - 1.5);
    assert_eq!(new.log_w, repaired.log_w);
    assert_eq!(repaired.set_log_w[2], -1.5);
    let mut another = set.clone();
    another.tera_type = dex::type_id("Grass").unwrap();
    let again = replay_set(&cx(&builder), &repaired, index, MonId(8), &another, -0.7, &mut stats).unwrap();
    assert_eq!(again.log_w, old.log_w - 0.7, "a second set repair replaces the first pool share");
    assert_eq!(again.history.last().unwrap().log_w, again.log_w);
    assert_eq!(again.set_log_w[2], -0.7);
    assert_eq!(stats.attempts, 0, "the newest anchor has no recorded segments to replay");
    turn(&mut builder, &mut real, "switch 3, move 2");
    *builder.lines.last_mut().unwrap() = "|turn|999".into();
    assert!(advance_capped(&cx(&builder), &mut repaired, 3, &mut stats).is_err());
    assert!(stats.attempts <= 3);
    assert!(new.introduces.load(Ordering::Relaxed));
    assert!(!old.introduces.load(Ordering::Relaxed), "a failed repair cannot mark the dead lineage's anchor");
}

#[test]
fn recovery_caps_both_recorded_replays_and_new_segment_searches() {
    let (mut builder, mut real) = fixture(|_, _| {});
    for _ in 0..2 {
        turn(&mut builder, &mut real, "move 2, move 4");
        builder.update().unwrap();
    }
    let p = &builder.particles[0];
    let mut stats = WorldStats::default();
    let set = &p.battle.teams().sides[1].sets[0];
    assert!(matches!(replay_set_capped(&cx(&builder), p, 0, MonId(6), set, 0.0, 1, &mut stats),
        Err(ReplaySetError::Interrupted)));
    assert_eq!((stats.attempts, stats.repair_replays), (1, 1));
    let snap = &p.history[1];
    let mut restored = p.clone();
    restored.battle = snap.battle.clone();
    restored.cursor = snap.cursor;
    restored.choice = snap.choice;
    restored.seg = snap.seg;
    restored.history = p.history[..1].to_vec();
    restored.trail = Replay::before(&p.trail, snap.seg);
    stats = WorldStats::default();
    let mut unlimited = restored.clone();
    assert!(advance_capped(&cx(&builder), &mut restored, 1, &mut stats).is_err());
    assert_eq!(stats.attempts, 1);
    stats = WorldStats::default();
    advance(&cx(&builder), &mut unlimited, &mut stats).unwrap();
    assert!(stats.attempts >= 2, "normal updates can replay all segments");
}

#[test]
fn redraws_knowing_tera_pay_no_second_share_and_leave_other_slots_unconditioned() {
    let (mut builder, _) = fixture(|pool, team| {
        for slot in [1, 2] {
            for _ in 0..2 { pool.add_set(&team.sets[slot]); }
            let mut water = team.sets[slot].clone();
            water.tera_type = dex::type_id("Water").unwrap();
            pool.add_set(&water);
        }
    });
    let mut p = builder.particles[0].clone();
    builder.push_lines(["|switch|p2a: Bastiodon|Bastiodon, L90, M|100/100", "|-terastallize|p2a: Bastiodon|Water"]);
    redraw_hidden(&cx(&builder), &mut p).unwrap();
    assert!(p.tera_known & (1 << 8) != 0);
    let before = p.log_w;
    p.battle.switch_in(MonId(8), 0, crate::event::EffectRef::None, true);
    p.battle.send_updates();
    retera(&cx(&builder), &mut p, &["|-terastallize|p2a: Bastiodon|Water".into()]).unwrap();
    assert_eq!(p.log_w, before);
    retera(&cx(&builder), &mut p, &["|-terastallize|p2b: Umbreon|Water".into()]).unwrap();
    assert_eq!(p.log_w, before + (1.0f64 / 4.0).ln(), "redrawing the bench does not condition an untouched lead");
}

#[test]
fn off_pool_tera_preserves_its_penalty_without_marking_the_world_patched() {
    let (builder, _) = fixture(|_, _| {});
    let mut p = builder.particles[0].clone();
    let before = p.log_w;
    retera(&cx(&builder), &mut p, &["|-terastallize|p2a: Latias|Water".into()]).unwrap();
    assert_eq!(p.log_w, before - REPAIR_LOG_PENALTY);
    assert!(!p.repaired);
    retera(&cx(&builder), &mut p, &["|-terastallize|p2a: Latias|Water".into()]).unwrap();
    assert_eq!(p.log_w, before - REPAIR_LOG_PENALTY);
    let mut p = builder.particles[0].clone();
    let mut patched = p.battle.teams().sides[1].sets[0].clone();
    patched.moves[0] = dex::MOVE_SLUDGEBOMB;
    let mut battle = p.history[0].battle.clone();
    battle.replace_hidden_set(SideId(0), MonId(6), &patched).unwrap();
    battle.start().unwrap();
    p.battle = battle;
    p.repaired = true;
    p.log_w -= REPAIR_LOG_PENALTY;
    let before = p.log_w;
    retera(&cx(&builder), &mut p, &["|-terastallize|p2a: Latias|Water".into()]).unwrap();
    assert_eq!(p.log_w, before, "a set patched outside the pool already paid the penalty");
}

#[test]
fn drawing_an_off_pool_tera_only_counts_the_weight_penalty() {
    let (mut builder, _) = fixture(|_, _| {});
    builder.push_lines(["|-terastallize|p2a: Latias|Water"]);
    let context = cx(&builder);
    let mut stats = WorldStats::default();
    let p = build(&context, None, builder.own.clone(), &builder.names, &mut SplitMix64(5), &mut stats).unwrap();
    assert_eq!(p.log_w, -REPAIR_LOG_PENALTY);
    assert!(!p.repaired);
}

#[test]
fn coverage_correction_is_in_the_restored_snapshots_and_survives_set_replay() {
    let (mut builder, mut real) = fixture(|pool, team| {
        for _ in 0..2 { pool.add_set(&team.sets[2]); }
        let mut water = team.sets[2].clone();
        water.tera_type = dex::type_id("Water").unwrap();
        pool.add_set(&water);
    });
    let anchor = builder.particles[0].history.last().unwrap().clone();
    turn(&mut builder, &mut real, "switch 3, move 2");
    builder.update().unwrap();
    let dead = builder.particles[0].clone();
    let (mut source, mut cached, mut stats, mut failure) = (None, Vec::new(), WorldStats::default(), String::new());
    let restored = backtrack(&cx(&builder), &[dead], 2, 100, &mut SplitMix64(7), &mut stats,
        &mut failure, &mut source, &mut cached);
    assert_eq!(restored.len(), 2, "{failure}");
    let second = &restored[1];
    let repaired_anchor = second.history.iter().position(|s| s.seg == anchor.seg).unwrap();
    assert!(second.history[repaired_anchor].log_w < anchor.log_w, "the second distinct draw pays the coverage correction before search");
    let set = &second.battle.teams().sides[1].sets[2];
    let replayed = replay_set(&cx(&builder), second, repaired_anchor, MonId(8), set, second.set_log_w[2], &mut stats).unwrap();
    assert_eq!(replayed.log_w, second.history.last().unwrap().log_w);
    assert!(replayed.log_w < anchor.log_w);
}

#[test]
fn cap_interrupted_repairs_can_succeed_on_a_later_update() {
    for during_replay in [true, false] {
        let (mut builder, mut real) = fixture(|pool, team| {
            let mut wrong = team.sets[0].clone();
            wrong.moves[0] = dex::MOVE_PSYCHIC;
            for _ in 0..300 { pool.add_set(&wrong); }
        });
        assert!(!builder.particles[0].battle.teams().sides[1].sets[0].moves.contains(&dex::MOVE_MISTBALL));
        builder.config.backtrack_budget = 0;
        builder.config.rebuild_budget = 0;
        turn(&mut builder, &mut real, "move 2, move 4");
        builder.update().unwrap();
        turn(&mut builder, &mut real, "move 1 +1, move 4");
        turn(&mut builder, &mut real, "move 2, move 4");
        let mut p = builder.particles.pop().unwrap();
        let mut stats = builder.stats;
        p.death = Some(advance(&cx(&builder), &mut p, &mut stats).unwrap_err());
        assert_eq!(p.seg, 2);
        let mut dead = vec![p];
        // Either interrupt two recorded replays, or leave only one attempt for two new segments.
        let cap = stats.attempts + if during_replay { 1 } else { 3 };
        assert!(repair_sets(&cx(&builder), &mut dead, cap, &mut stats, &mut None, &mut Vec::new()).is_empty());
        assert_eq!(stats.attempts, cap);
        assert_eq!((stats.set_repair_attempts, stats.set_repairs), (1, 0));
        assert_eq!(stats.repair_replays, if during_replay { 1 } else { 2 });
        assert!(dead[0].tried.is_empty(), "an interrupted candidate has not been rejected");
        builder.stats = stats;
        builder.graveyard = dead;
        builder.update().unwrap_or_else(|e| panic!("during_replay={during_replay}: {e}; {}", builder.last_failure()));
        assert_eq!((builder.stats.set_repair_attempts, builder.stats.set_repairs), (2, 1));
        assert_eq!(builder.particles[0].battle.request_json(0), real.request_json(0));
        assert!(builder.graveyard.is_empty());
    }
}

#[test]
fn set_repair_replaces_the_old_off_pool_tera_weight() {
    let (mut builder, mut real) = fixture(|pool, team| {
        let mut wrong = team.sets[0].clone();
        wrong.moves[0] = dex::MOVE_PSYCHIC;
        for _ in 0..300 { pool.add_set(&wrong); }
    });
    builder.config.backtrack_budget = 0;
    builder.config.rebuild_budget = 0;
    real.set_hidden_tera_type(SideId(0), MonId(6), dex::type_id("Water").unwrap()).unwrap();
    builder.push_choice("move 1, move 3");
    real.log.reset();
    real.choose(0, "move 1, move 3").unwrap();
    real.choose(1, "move 2 terastallize, move 4").unwrap();
    push(&mut builder, &real);
    builder.update().unwrap();
    assert_eq!(builder.particles[0].set_log_w[0], -REPAIR_LOG_PENALTY);
    turn(&mut builder, &mut real, "move 1 +1, move 4");
    builder.update().unwrap_or_else(|e| panic!("{e}; {}", builder.last_failure()));
    assert_eq!(builder.stats.set_repairs, 1);
    let repaired = &builder.particles[0];
    let fresh = build(&cx(&builder), None, builder.own.clone(), &builder.names, &mut SplitMix64(5),
        &mut WorldStats::default()).unwrap();
    let mut repaired_set = repaired.battle.teams().sides[1].sets[0].clone();
    let mut fresh_set = fresh.battle.teams().sides[1].sets[0].clone();
    repaired_set.moves.sort_unstable_by_key(|m| m.0);
    fresh_set.moves.sort_unstable_by_key(|m| m.0);
    assert_eq!(repaired_set, fresh_set);
    assert_eq!(repaired.set_log_w, fresh.set_log_w);
    assert_eq!(repaired.set_log_w[0], -REPAIR_LOG_PENALTY);
    // These protected turns need one attempt each, so both worlds pay only the set penalty.
    assert_eq!(repaired.log_w, fresh.log_w);
    for s in &repaired.history {
        assert_eq!(s.set_log_w[0], fresh.set_log_w[0]);
        assert_eq!(s.log_w, fresh.log_w);
    }
}

#[test]
fn a_successful_update_releases_the_graveyard() {
    let (mut builder, real) = fixture(|_, _| {});
    builder.graveyard.push(builder.particles[0].clone());
    builder.push_request(&real.request_json(0).unwrap());
    builder.update().unwrap();
    assert!(builder.graveyard.is_empty());
}

#[test]
fn backtracking_checks_contradictions_from_the_current_illusion_view() {
    let pool = fixture_pool();
    let config = WorldConfig::default();
    let mut tracker = RevealTracker::new();
    let mut observed = lines("|teamsize|p2|3
        |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
        |switch|p2b: Umbreon|Umbreon, L85, M|100/100
        |turn|1");
    tracker.feed_all(observed.iter().map(String::as_str));
    let initial = Ctx { lines: &observed, ..context(&tracker, &pool, &config) };
    let h = hypotheses(&initial).into_iter().find(|h| h.user == "Zoroark").unwrap();
    let mut stats = WorldStats::default();
    let mut dead = build(&initial, Some(h), Arc::new(TeamDef::unpack(&[BL, M].join("]")).unwrap()),
        &["A".into(), "B".into()], &mut SplitMix64(7), &mut stats).unwrap();
    dead.battle.start().unwrap();
    dead.cursor = observed.len();
    dead.seg = 1;
    let snap = Arc::new(Snapshot::of(&dead));
    snap.introduces.store(true, Ordering::Relaxed);
    dead.history.push(snap);
    let new = lines("|replace|p2a: Zoroark|Zoroark, L84, F
        |move|p2a: Zoroark|Psychic|p1a: Blastoise");
    tracker.feed_all(new.iter().map(String::as_str));
    observed.extend(new);
    let current = Ctx { lines: &observed, ..context(&tracker, &pool, &config) };
    let stale = Ctx { seen: &dead.hypothesis.as_ref().unwrap().view,
        hypothesis: dead.hypothesis.as_deref(), ..current };
    assert!(contradicted(&stale, &dead.battle).is_empty());
    assert_eq!(contradicted(&current, &dead.battle), ["Zoroark"]);
    let (mut source, mut cached, mut stats, mut failure) = (None, Vec::new(), WorldStats::default(), String::new());
    assert!(backtrack(&current, &[dead], 1, 100, &mut SplitMix64(9), &mut stats,
        &mut failure, &mut source, &mut cached).is_empty());
    assert_eq!(stats.restores, 0, "a contradiction already in play rules out the death boundary");
}

#[test]
fn introducing_illusion_mid_advance_enables_entry_capture() {
    let own = Arc::new(TeamDef::unpack(&[BL, M].join("]")).unwrap());
    let opp = Arc::new(TeamDef::unpack(&[L, U, Z, B].join("]")).unwrap());
    let pool = Arc::new(fixture_pool());
    let mut real = Battle::from_team_defs([1, 2, 3, 4], own.clone(), opp, ["A".into(), "B".into()],
        DrawLog::default()).unwrap();
    real.start().unwrap();
    let config = WorldConfig { particles: 1, rebuild_budget: 1, fresh_per_update: 0, ..WorldConfig::default() };
    let mut builder = WorldBuilder::new(SideId(0), own, ["A".into(), "B".into()], pool, config, 5).unwrap();
    push(&mut builder, &real);
    builder.update().unwrap();
    let mut p = builder.particles[0].clone();
    for (mon, packed) in [(MonId(8), M), (MonId(9), B)] {
        p.battle.replace_hidden_set(SideId(0), mon, &TeamDef::unpack(packed).unwrap().sets[0]).unwrap();
    }
    assert!(!p.battle.teams().sides[1].sets.iter().any(|s| s.ability == dex::ABILITY_ILLUSION));
    // Reserve the real disguise so introduction draws into the preceding slot.
    p.battle.mark_revealed(MonId(9));
    builder.push_choice("move 4 +1, move 1");
    real.log.reset();
    real.choose(0, "move 4 +1, move 1").unwrap();
    real.choose(1, "switch 3, move 2").unwrap();
    push(&mut builder, &real);
    assert!(builder.tracker.find(SideId(1), "Zoroark").is_some());
    let mut stats = WorldStats::default();
    advance(&cx(&builder), &mut p, &mut stats).unwrap_or_else(|d| panic!("{d:?}: {}", p.why));
    assert_eq!(stats.swaps, 1);
    assert!(p.battle.log.track_entries);
    assert!(p.battle.log.entry_mons.iter().any(|&(_, mon)| set_name(&p.battle, mon) == "Zoroark"));
}

#[test]
fn an_introduced_set_drawn_knowing_tera_pays_no_second_share() {
    let (mut builder, _) = fixture(|pool, team| {
        for _ in 0..300 { pool.add_set(&team.sets[2]); }
        let poison = TeamDef::unpack(Z).unwrap().sets[0].clone();
        pool.add_set(&poison);
        let water = SetDef { tera_type: dex::type_id("Water").unwrap(), ..poison };
        for _ in 0..3 { pool.add_set(&water); }
    });
    let mut p = builder.particles[0].clone();
    assert_eq!(set_name(&p.battle, MonId(8)), "Bastiodon");
    let window = lines("|switch|p2a: Zoroark|Zoroark, L84, F|100/100
        |-terastallize|p2a: Zoroark|Poison");
    builder.push_lines(window.iter().map(String::as_str));
    let mut stats = WorldStats::default();
    introduce(&cx(&builder), &mut p, &window, &mut stats).unwrap();
    assert_eq!(stats.swaps, 1);
    assert!(p.tera_known & (1 << 8) != 0);
    let before = p.log_w;
    p.battle.switch_in(MonId(8), 0, crate::event::EffectRef::None, true);
    p.battle.send_updates();
    retera(&cx(&builder), &mut p, &[window[1].clone()]).unwrap();
    assert_eq!(p.log_w, before);
}

#[test]
fn a_tera_only_patch_does_not_keep_building_with_an_illusion_pool() {
    let (mut builder, mut real) = fixture(|pool, team| {
        for _ in 0..300 { pool.add_set(&team.sets[2]); }
        pool.add_team(Z).unwrap();
    });
    builder.config.fresh_per_update = 1;
    real.set_hidden_tera_type(SideId(0), MonId(6), dex::type_id("Water").unwrap()).unwrap();
    let builds = builder.stats.builds;
    turn(&mut builder, &mut real, "move 2 terastallize, move 4");
    builder.update().unwrap();
    assert!(!builder.particles[0].repaired);
    assert_eq!(builder.stats.builds, builds);
    turn(&mut builder, &mut real, "move 2, move 4");
    builder.update().unwrap();
    assert_eq!(builder.stats.builds, builds, "the Tera-only patch does not ask for a fresh build each update");
}

#[test]
fn a_set_repair_knowing_tera_pays_no_second_share() {
    let (mut builder, _) = fixture(|pool, team| {
        let water = SetDef { tera_type: dex::type_id("Water").unwrap(), ..team.sets[0].clone() };
        pool.add_set(&water);
    });
    let p = builder.particles[0].clone();
    builder.tracker.feed("|-terastallize|p2a: Latias|Water");
    let mut set = p.battle.teams().sides[1].sets[0].clone();
    set.tera_type = dex::type_id("Water").unwrap();
    let mut repaired = replay_set(&cx(&builder), &p, 0, MonId(6), &set, -0.7, &mut WorldStats::default()).unwrap();
    assert!(repaired.tera_known & (1 << 6) != 0);
    let before = repaired.log_w;
    retera(&cx(&builder), &mut repaired, &["|-terastallize|p2a: Latias|Water".into()]).unwrap();
    assert_eq!(repaired.log_w, before);
}

#[test]
fn skipping_tera_variants_of_the_old_set_does_not_hide_new_repairs() {
    let (mut builder, _) = fixture(|pool, team| {
        for name in ["Fire", "Water", "Grass", "Electric", "Ice", "Fighting", "Poison", "Ground", "Flying", "Psychic"] {
            let variant = SetDef { tera_type: dex::type_id(name).unwrap(), ..team.sets[0].clone() };
            for _ in 0..3 { pool.add_set(&variant); }
        }
        let alternate = SetDef { item: dex::ITEM_CHOICESCARF, ..team.sets[0].clone() };
        pool.add_set(&alternate);
    });
    builder.tracker.feed("|-terastallize|p2a: Latias|Dark");
    let context = cx(&builder);
    let p = &builder.particles[0];
    let mut old = TeamDef::unpack(L).unwrap().sets[0].clone();
    old.tera_type = dex::type_id("Dark").unwrap();
    let candidates = repair_candidates(&context, p, MonId(6), &constraints(&context, 0), &old, 1, &mut SplitMix64(5));
    assert!(candidates.iter().any(|(_, set, _)| set.item == dex::ITEM_CHOICESCARF),
        "all more frequent records become the old set when given its revealed Tera");
}
