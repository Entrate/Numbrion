//! Tests for the hazards_screens batch.
//!
//! * Callback vectors (`vectors.tsv`) come from running the real pinned handler functions
//!   (`tools/probes/hazards_screens/vectors.mjs`) and run today: they use direct hook calls
//!   (`Battle::call_hook`) with hand-built frames and only the already-implemented queries.
//! * `scenarios.txt` holds complete battles replayed on the pinned Showdown
//!   (`tools/probes/hazards_screens/scenarios.mjs`): exact log lines and PRNG state after every
//!   decision boundary. They need the unfinished battle core and are `#[ignore = "needs core"]`.
use super::*;
use crate::{
    actions::MoveHandle,
    dex::{Category, HookId},
    event::{CallArgs, EventFrame},
    log::{LogEntry, LogSink, LogView, MoveLineEdit},
    state::{CellRef, mon_flags, scratch::*},
};

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// Records every protocol line as `command|arg|...|[tag]` (static data only, test-side formatting).
#[derive(Default)]
struct Trace(Vec<String>);
impl LogSink for Trace {
    const ENABLED: bool = true;
    fn emit(&mut self, _: LogView<'_>, e: LogEntry<'_>) {
        fn arg(a: &LogArg<'_>) -> String {
            match a {
                LogArg::Text(s) => (*s).into(),
                LogArg::Mon(m) => format!("mon{}", m.0),
                LogArg::Side(s) => format!("side{}", s.0),
                LogArg::Effect(EffectRef::Dex(id)) => dex::effect(*id).name.into(),
                other => format!("{other:?}"),
            }
        }
        let mut parts = vec![e.command.to_owned()];
        parts.extend(e.args.iter().map(arg));
        parts.extend(e.tags.iter().map(|t| match t {
            LogTag::From(EffectRef::Dex(id)) => format!("[from] {}", dex::effect(*id).name),
            LogTag::Of(m) => format!("[of] mon{}", m.0),
            LogTag::Bare(s) => format!("[{s}]"),
            other => format!("{other:?}"),
        }));
        self.0.push(parts.join("|"));
    }
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {
        panic!("unexpected move edit")
    }
}

fn battle<L: LogSink>(log: L) -> Battle<L> {
    let packed = "Pikachu|||static|thunderbolt|Serious||M|||100|,,,,,Electric";
    let mut b = Battle::with_log([1, 2, 3, 4], packed, packed, log).unwrap();
    b.state.sides[0].active = [MonId(0), MonId::NONE];
    b.state.sides[1].active = [MonId(6), MonId::NONE];
    b.state.pokemon[0].flags |= mon_flags::ACTIVE;
    b.state.pokemon[6].flags |= mon_flags::ACTIVE;
    b.scratch.moves[0] = Some(active_move());
    b.scratch.frames[0] = Some(frame(4096));
    b.scratch.current_frame = 0;
    b
}
fn frame(modifier: u32) -> EventFrame {
    EventFrame {
        event: EventId::ModifyDamage,
        target: EventArg::Undefined,
        source: EventArg::Undefined,
        source_effect: EffectRef::None,
        relay: Relay::Number(100.0),
        modifier,
        modifier_present: true,
        has_relay: true,
        fast_exit: false,
        args: [EventArg::Undefined; 4],
        arg_count: 0,
    }
}
fn active_move() -> ActiveMove {
    let m = dex::move_data(dex::MOVE_BRICKBREAK);
    ActiveMove {
        flags: m.flags,
        traits: m.traits,
        runtime_flags: 0,
        total_damage: 0,
        hit_targets: [MonId::NONE; 4],
        hit_target_len: 0,
        id: m.id,
        source_effect: EffectRef::None,
        type_changer_boosted: EffectId::NONE,
        ruined_stats: [MonId::NONE; 4],
        base_power: f64::from(m.base_power),
        accuracy: MoveAccuracy::Percent(100.0),
        hit_data: [HitData::default(); 12],
        self_boosts: OrderedBoosts::default(),
        effects: MoveEffectsScratch {
            base: &m.effects,
            boosts: OrderedBoosts::default(),
            status: m.effects.status,
            volatile_status: m.effects.volatile_status,
            side_condition: m.effects.side_condition,
            slot_condition: m.effects.slot_condition,
            weather: m.effects.weather,
            terrain: m.effects.terrain,
            pseudo_weather: m.effects.pseudo_weather,
            chance: None,
            heal: m.effects.heal,
            force_switch: m.effects.force_switch,
            self_switch: m.effects.self_switch,
            suppressed_hooks: 0,
        },
        self_effect: None,
        secondaries: [None; 4],
        secondary_count: 0,
        secondaries_present: false,
        recoil: m.recoil,
        drain: m.drain,
        damage: m.damage,
        self_destruct: m.self_destruct,
        offensive_stat: m.offensive_stat,
        defensive_stat: m.defensive_stat,
        offensive_target: m.offensive_target,
        ignore_immunity_types: 0,
        move_type: m.move_type,
        category: Category::Physical,
        target: m.target,
        priority: m.priority,
        crit_ratio: m.crit_ratio,
        hit: 0,
        multihit: m.multihit,
    }
}
/// Allocates `id`'s state on `holder`, links it into the side list (when a Side) and makes it the
/// current hook state, as the event system does before invoking a callback.
fn install<L: LogSink>(b: &mut Battle<L>, id: EffectId, holder: Holder) -> CellRef {
    let cell = b.state.effects.alloc(holder, holder, id, 0);
    if holder.0 >= 12 && holder.0 < 14 {
        b.state.sides[(holder.0 - 12) as usize]
            .conditions
            .push(cell);
    }
    let r = b.state.effects.capture(cell);
    b.scratch.current_state = Some(r);
    r
}
fn call<L: LogSink>(b: &mut Battle<L>, hook: HookId, args: &[EventArg]) -> Relay {
    let mut values = [EventArg::Undefined; 4];
    values[..args.len()].copy_from_slice(args);
    b.call_hook(
        hook,
        CallArgs {
            values,
            len: args.len() as u8,
        },
    )
}
fn modifier<L: LogSink>(b: &Battle<L>) -> u32 {
    b.scratch.frames[0].unwrap().modifier
}
fn mon(m: u8) -> EventArg {
    EventArg::Holder(Holder::mon(MonId(m)))
}
fn side(s: u8) -> EventArg {
    EventArg::Holder(Holder::side(SideId(s)))
}
fn tsv(text: &'static str) -> impl Iterator<Item = Vec<&'static str>> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split('\t').collect())
}

// ---------------------------------------------------------------------------------------------
// Pure data / helper checks
// ---------------------------------------------------------------------------------------------

#[test]
fn type_ids_match_dex() {
    for (id, name) in [
        (TYPE_FIGHTING, "Fighting"),
        (TYPE_POISON, "Poison"),
        (TYPE_STEEL, "Steel"),
        (TYPE_FIRE, "Fire"),
        (TYPE_WATER, "Water"),
    ] {
        assert_eq!(dex::TYPE_NAMES[id.0 as usize - 1], name);
    }
}

#[test]
fn boost_objects_keep_js_insertion_order() {
    let o = boosts_of(&[(STAT_ATK, 1), (STAT_SPE, 1)]);
    assert_eq!((o.order[0], o.order[1], o.len), (0, 4, 2));
    assert_eq!((o.values[0], o.values[4], o.present), (1, 1, 0b10001));
    let o = boosts_of(&[(STAT_SPE, -1)]);
    assert_eq!(
        (o.order[0], o.len, o.values[4], o.present),
        (4, 1, -1, 0b10000)
    );
}

#[test]
fn side_helpers() {
    assert_eq!(side_of(Holder::side(SideId(0))), SideId(0));
    assert_eq!(side_of(Holder::side(SideId(1))), SideId(1));
    assert_eq!(foe_of(SideId(0)), SideId(1));
    assert_eq!(foe_of(SideId(1)), SideId(0));
}

// ---------------------------------------------------------------------------------------------
// Court Change (data/moves.ts:3066-3099): list order, target rewrite, success flag
// ---------------------------------------------------------------------------------------------

fn conditions(b: &Battle<NoLog>, s: usize) -> Vec<(EffectId, Holder)> {
    b.state.sides[s]
        .conditions
        .as_slice()
        .iter()
        .map(|c| {
            let cell = &b.state.effects.cells[c.0 as usize];
            (cell.id, cell.target)
        })
        .collect()
}
use crate::log::NoLog;

#[test]
fn court_change_swaps_allow_listed_states_in_insertion_order() {
    let mut b = battle(NoLog);
    let (s0, s1) = (Holder::side(SideId(0)), Holder::side(SideId(1)));
    // JS object order: Reflect, Wide Guard (not swapped), Spikes on side 0; Tailwind, Wide Guard, Rock on side 1.
    for id in [
        dex::CONDITION_REFLECT,
        dex::CONDITION_WIDEGUARD,
        dex::CONDITION_SPIKES,
    ] {
        install(&mut b, id, s0);
    }
    for id in [
        dex::CONDITION_TAILWIND,
        dex::CONDITION_WIDEGUARD,
        dex::CONDITION_STEALTHROCK,
    ] {
        install(&mut b, id, s1);
    }
    let reflect_cell = b.state.sides[0].conditions.cells[0];
    let reflect_generation = b.state.effects.cells[reflect_cell.0 as usize].generation;
    assert!(swap_side_conditions(&mut b, SideId(0)));
    assert_eq!(
        conditions(&b, 0),
        vec![
            (dex::CONDITION_WIDEGUARD, s0),
            (dex::CONDITION_TAILWIND, s0),
            (dex::CONDITION_STEALTHROCK, s0)
        ]
    );
    assert_eq!(
        conditions(&b, 1),
        vec![
            (dex::CONDITION_WIDEGUARD, s1),
            (dex::CONDITION_REFLECT, s1),
            (dex::CONDITION_SPIKES, s1)
        ]
    );
    // Moved states keep their identity (object, generation) -- only the target changes.
    let moved = b.state.sides[1].conditions.cells[1];
    assert_eq!(moved, reflect_cell);
    assert_eq!(
        b.state.effects.cells[moved.0 as usize].generation,
        reflect_generation
    );
    // Swapping again restores the original arrangement of the swapped ids.
    assert!(swap_side_conditions(&mut b, SideId(1)));
    assert_eq!(
        conditions(&b, 0),
        vec![
            (dex::CONDITION_WIDEGUARD, s0),
            (dex::CONDITION_REFLECT, s0),
            (dex::CONDITION_SPIKES, s0)
        ]
    );
    // PRNG untouched.
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}

#[test]
fn court_change_fails_without_swappable_conditions() {
    let mut b = battle(NoLog);
    install(&mut b, dex::CONDITION_WIDEGUARD, Holder::side(SideId(0)));
    assert!(!swap_side_conditions(&mut b, SideId(0)));
    assert_eq!(
        conditions(&b, 0),
        vec![(dex::CONDITION_WIDEGUARD, Holder::side(SideId(0)))]
    );
    assert!(conditions(&b, 1).is_empty());
    let r = call(
        &mut b,
        dex::HOOK_MOVE_COURTCHANGE_ONHITFIELD,
        &[mon(6), mon(0), EventArg::Move(0)],
    );
    assert_eq!(r, Relay::Bool(false));
}

// ---------------------------------------------------------------------------------------------
// Layered hazards: effectState.layers
// ---------------------------------------------------------------------------------------------

#[test]
fn spikes_and_toxic_spikes_layer_state_and_logs() {
    for (id, start, restart, max, text) in [
        (
            dex::CONDITION_SPIKES,
            dex::HOOK_CONDITION_SPIKES_ONSIDESTART,
            dex::HOOK_CONDITION_SPIKES_ONSIDERESTART,
            3,
            "Spikes",
        ),
        (
            dex::CONDITION_TOXICSPIKES,
            dex::HOOK_CONDITION_TOXICSPIKES_ONSIDESTART,
            dex::HOOK_CONDITION_TOXICSPIKES_ONSIDERESTART,
            2,
            "move: Toxic Spikes",
        ),
    ] {
        let mut b = battle(Trace::default());
        let r = install(&mut b, id, Holder::side(SideId(1)));
        let args = [side(1), mon(0), EventArg::Undefined];
        assert_eq!(call(&mut b, start, &args), Relay::Undefined);
        let c = b.state.effects.cells[r.cell.0 as usize];
        assert_eq!(c.payload.words[0], 1);
        assert_ne!(c.present & (1 << 8), 0, "layers presence bit");
        for layers in 2..=max {
            assert_eq!(call(&mut b, restart, &args), Relay::Undefined);
            assert_eq!(
                b.state.effects.cells[r.cell.0 as usize].payload.words[0],
                layers
            );
        }
        let logged = b.log.0.len();
        // The next layer is refused with `false`, prints nothing and keeps the count.
        assert_eq!(call(&mut b, restart, &args), Relay::Bool(false));
        assert_eq!(b.log.0.len(), logged);
        assert_eq!(
            b.state.effects.cells[r.cell.0 as usize].payload.words[0],
            max
        );
        let line = format!("-sidestart|side1|{text}");
        assert_eq!(b.log.0, vec![line; max as usize]);
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}

#[test]
fn side_start_and_end_lines_are_literal_call_site_text() {
    let table: [(HookId, Option<HookId>, &str); 6] = [
        (
            dex::HOOK_CONDITION_REFLECT_ONSIDESTART,
            Some(dex::HOOK_CONDITION_REFLECT_ONSIDEEND),
            "Reflect",
        ),
        (
            dex::HOOK_CONDITION_LIGHTSCREEN_ONSIDESTART,
            Some(dex::HOOK_CONDITION_LIGHTSCREEN_ONSIDEEND),
            "move: Light Screen",
        ),
        (
            dex::HOOK_CONDITION_AURORAVEIL_ONSIDESTART,
            Some(dex::HOOK_CONDITION_AURORAVEIL_ONSIDEEND),
            "move: Aurora Veil",
        ),
        (
            dex::HOOK_CONDITION_TAILWIND_ONSIDESTART,
            Some(dex::HOOK_CONDITION_TAILWIND_ONSIDEEND),
            "move: Tailwind",
        ),
        (
            dex::HOOK_CONDITION_STEALTHROCK_ONSIDESTART,
            None,
            "move: Stealth Rock",
        ),
        (
            dex::HOOK_CONDITION_STICKYWEB_ONSIDESTART,
            None,
            "move: Sticky Web",
        ),
    ];
    for (start, end, text) in table {
        let mut b = battle(Trace::default());
        let id = dex::HOOKS[start.0 as usize].effect;
        install(&mut b, id, Holder::side(SideId(0)));
        let args = [side(0), mon(0), EventArg::Undefined];
        assert_eq!(call(&mut b, start, &args), Relay::Undefined);
        let mut want = vec![format!("-sidestart|side0|{text}")];
        if let Some(end) = end {
            assert_eq!(call(&mut b, end, &[side(0)]), Relay::Undefined);
            want.push(format!("-sideend|side0|{text}"));
        }
        assert_eq!(b.log.0, want);
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}

// ---------------------------------------------------------------------------------------------
// Pinned-handler vectors (tools/probes/hazards_screens/vectors.mjs)
// ---------------------------------------------------------------------------------------------

fn screen_vector(row: &[&str], b: &mut Battle<NoLog>) -> (u32, Relay) {
    let initial: u32 = row[2].parse().unwrap();
    let category = match row[3] {
        "Physical" => Category::Physical,
        "Special" => Category::Special,
        "Status" => Category::Status,
        _ => panic!("category"),
    };
    let flag = |i: usize| row[i] == "1";
    let (crit, infiltrates, same_side, target_is_source) = (flag(4), flag(5), flag(6), flag(7));
    // The screen lives on side 0; `sameSide` puts the defender there (mon 0), else on side 1 (mon 6).
    let (defender, other) = if same_side { (0u8, 6u8) } else { (6u8, 0u8) };
    let attacker = if target_is_source { defender } else { other };
    let hook = match row[1] {
        "reflect" => dex::HOOK_CONDITION_REFLECT_ONANYMODIFYDAMAGE,
        "lightscreen" => dex::HOOK_CONDITION_LIGHTSCREEN_ONANYMODIFYDAMAGE,
        "auroraveil" => dex::HOOK_CONDITION_AURORAVEIL_ONANYMODIFYDAMAGE,
        _ => panic!("screen"),
    };
    let id = dex::HOOKS[hook.0 as usize].effect;
    // Other screens on the defender's side (read by Aurora Veil), then the screen under test last so
    // that its cell is the current hook state, as the event system arranges.
    if flag(8) {
        install(
            b,
            dex::CONDITION_REFLECT,
            Holder::side(SideId(defender / 6)),
        );
    }
    if flag(9) {
        install(
            b,
            dex::CONDITION_LIGHTSCREEN,
            Holder::side(SideId(defender / 6)),
        );
    }
    install(b, id, Holder::side(SideId(0)));
    b.scratch.moves[0].as_mut().unwrap().category = category;
    b.scratch.moves[0].as_mut().unwrap().runtime_flags = if infiltrates {
        move_runtime::INFILTRATES
    } else {
        0
    };
    b.move_hit_data(MonId(defender), MoveHandle(0)).crit = crit;
    b.scratch.frames[0] = Some(frame(initial));
    let relay = call(
        b,
        hook,
        &[
            EventArg::Number(100.0),
            mon(attacker),
            mon(defender),
            EventArg::Move(0),
        ],
    );
    (modifier(b), relay)
}

fn run_screen_vectors(screens: &[&str]) -> usize {
    let mut count = 0;
    for row in tsv(include_str!("vectors.tsv")).filter(|r| r[0] == "S" && screens.contains(&r[1])) {
        let mut b = battle(NoLog);
        let (m, relay) = screen_vector(&row, &mut b);
        assert_eq!(m, row[10].parse::<u32>().unwrap(), "{row:?}");
        assert_eq!(relay, Relay::Undefined, "{row:?}");
        assert_eq!(b.seed(), [1, 2, 3, 4]);
        count += 1;
    }
    count
}

#[test]
fn reflect_and_light_screen_weakening_matches_pinned_handlers() {
    assert_eq!(run_screen_vectors(&["reflect", "lightscreen"]), 192);
}

/// Aurora Veil asks `getSideCondition`, an unfinished core mutator.
#[test]
#[ignore = "needs core"]
fn aurora_veil_weakening_matches_pinned_handler() {
    assert_eq!(run_screen_vectors(&["auroraveil"]), 384);
}

#[test]
fn tailwind_doubles_speed_through_chain_modify() {
    let mut count = 0;
    for row in tsv(include_str!("vectors.tsv")).filter(|r| r[0] == "T") {
        let mut b = battle(NoLog);
        install(&mut b, dex::CONDITION_TAILWIND, Holder::side(SideId(0)));
        b.scratch.frames[0] = Some(frame(row[1].parse().unwrap()));
        let relay = call(
            &mut b,
            dex::HOOK_CONDITION_TAILWIND_ONMODIFYSPE,
            &[EventArg::Number(100.0), mon(0)],
        );
        assert_eq!(relay, Relay::Undefined);
        assert_eq!(modifier(&b), row[2].parse::<u32>().unwrap(), "{row:?}");
        assert_eq!(b.seed(), [1, 2, 3, 4]);
        count += 1;
    }
    assert_eq!(count, 3);
}

#[test]
fn duration_callbacks_match_pinned_handlers() {
    let mut count = 0;
    for row in tsv(include_str!("vectors.tsv")).filter(|r| r[0] == "D") {
        let (has_source, clay) = (row[2] == "1", row[3] == "1");
        let hook = match row[1] {
            "reflect" => dex::HOOK_CONDITION_REFLECT_DURATIONCALLBACK,
            "lightscreen" => dex::HOOK_CONDITION_LIGHTSCREEN_DURATIONCALLBACK,
            "auroraveil" => dex::HOOK_CONDITION_AURORAVEIL_DURATIONCALLBACK,
            "tailwind" => dex::HOOK_CONDITION_TAILWIND_DURATIONCALLBACK,
            "trickroom" => dex::HOOK_CONDITION_TRICKROOM_DURATIONCALLBACK,
            _ => panic!("condition"),
        };
        let mut b = battle(Trace::default());
        if clay {
            b.state.pokemon[0].item = dex::ITEM_LIGHTCLAY;
        }
        let source = if has_source {
            mon(0)
        } else {
            EventArg::Undefined
        };
        // Sides: durationCallback.call(battle, side.active[0], source, sourceEffect) (side.ts:431);
        // fields: durationCallback.call(battle, source, source, sourceEffect) (field.ts:206).
        let args: [EventArg; 3] = if row[1] == "trickroom" {
            [source, source, EventArg::Undefined]
        } else {
            [mon(0), source, EventArg::Undefined]
        };
        let relay = call(&mut b, hook, &args);
        assert_eq!(relay, Relay::Number(row[4].parse().unwrap()), "{row:?}");
        assert!(
            b.log.0.is_empty(),
            "Persistent is outside the generated ability scope"
        );
        assert_eq!(b.seed(), [1, 2, 3, 4]);
        count += 1;
    }
    assert_eq!(count, 20);
}

// ---------------------------------------------------------------------------------------------
// Whole-battle replays against the pinned Showdown
// ---------------------------------------------------------------------------------------------

struct Step {
    label: &'static str,
    choices: [&'static str; 2],
    seed_after: [u16; 4],
    log: Vec<&'static str>,
}
struct Scenario {
    name: &'static str,
    seed: [u16; 4],
    teams: [&'static str; 2],
    steps: Vec<Step>,
}
fn parse_seed(text: &str) -> [u16; 4] {
    let v: Vec<u16> = text.split(',').map(|n| n.parse().unwrap()).collect();
    v.try_into().unwrap()
}
fn parse_scenarios(text: &'static str) -> Vec<Scenario> {
    let mut out: Vec<Scenario> = Vec::new();
    for line in text.lines() {
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "scenario" => out.push(Scenario {
                name: rest,
                seed: [0; 4],
                teams: [""; 2],
                steps: Vec::new(),
            }),
            "seed" => out.last_mut().unwrap().seed = parse_seed(rest),
            "p1" => out.last_mut().unwrap().teams[0] = rest,
            "p2" => out.last_mut().unwrap().teams[1] = rest,
            "step" => {
                let mut f = rest.split('\t');
                let label = f.next().unwrap();
                let c1 = f.next().unwrap_or("");
                let c2 = f.next().unwrap_or("");
                out.last_mut().unwrap().steps.push(Step {
                    label,
                    choices: [c1, c2],
                    seed_after: [0; 4],
                    log: Vec::new(),
                });
            }
            "seed_after" => {
                out.last_mut().unwrap().steps.last_mut().unwrap().seed_after = parse_seed(rest)
            }
            "log" => out
                .last_mut()
                .unwrap()
                .steps
                .last_mut()
                .unwrap()
                .log
                .push(rest),
            _ => panic!("unknown scenario record {line:?}"),
        }
    }
    out
}

#[test]
fn scenario_file_is_well_formed() {
    let scenarios = parse_scenarios(include_str!("scenarios.txt"));
    assert_eq!(scenarios.len(), SCENARIOS);
    for sc in &scenarios {
        assert_eq!(sc.steps[0].label, "start");
        assert!(sc.steps.len() > 2, "{}", sc.name);
        assert!(sc.steps.iter().all(|s| !s.log.is_empty()), "{}", sc.name);
    }
}
const SCENARIOS: usize = 15;

/// Exact log lines and PRNG state after every decision boundary of the directed battles, which
/// together reach every file in the batch. Set HAZARDS_SCENARIO=<name> to run just one.
#[test]
#[ignore = "needs core"]
fn scenarios_match_pinned_showdown() {
    use crate::log::TextLog;
    let only = std::env::var("HAZARDS_SCENARIO").ok();
    for sc in parse_scenarios(include_str!("scenarios.txt")) {
        if only.as_deref().is_some_and(|n| n != sc.name) {
            continue;
        }
        let mut b = Battle::from_players(
            sc.seed,
            ("Alice", sc.teams[0]),
            ("Bob", sc.teams[1]),
            TextLog::default(),
        )
        .unwrap();
        b.start().unwrap();
        for step in &sc.steps {
            for (side, choice) in step.choices.iter().enumerate() {
                if !choice.is_empty() {
                    b.choose(side, choice).unwrap_or_else(|e| {
                        panic!(
                            "{} {}: side {side} {choice:?} rejected: {e:?}",
                            sc.name, step.label
                        )
                    });
                }
            }
            let mut got = Vec::new();
            b.drain_log(&mut got);
            let got: Vec<String> = got
                .into_iter()
                .map(|l| {
                    if l.starts_with("|t:|") && l[4..].bytes().all(|c| c.is_ascii_digit()) {
                        "|t:|".to_owned()
                    } else {
                        l
                    }
                })
                .collect();
            assert_eq!(got, step.log, "{} {} log", sc.name, step.label);
            assert_eq!(b.seed(), step.seed_after, "{} {} seed", sc.name, step.label);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Move callbacks that decide before touching an unfinished core mutator
// ---------------------------------------------------------------------------------------------

#[test]
fn raging_bull_type_follows_the_paldean_forme() {
    let ty = |name: &str| dex::type_id(name).unwrap();
    for (species, expected) in [
        (dex::SPECIES_TAUROSPALDEACOMBAT, Some("Fighting")),
        (dex::SPECIES_TAUROSPALDEABLAZE, Some("Fire")),
        (dex::SPECIES_TAUROSPALDEAAQUA, Some("Water")),
        (dex::SPECIES_TAUROS, None),
        (dex::SPECIES_PIKACHU, None),
    ] {
        let mut b = battle(NoLog);
        b.state.pokemon[0].species = species;
        let before = b.scratch.moves[0].unwrap().move_type;
        let relay = call(
            &mut b,
            dex::HOOK_MOVE_RAGINGBULL_ONMODIFYTYPE,
            &[
                EventArg::Relay(Relay::ActiveMove(0)),
                mon(0),
                mon(6),
                EventArg::Effect(EffectRef::ActiveMove(0)),
            ],
        );
        assert_eq!(relay, Relay::Undefined);
        let after = b.scratch.moves[0].unwrap().move_type;
        assert_eq!(after, expected.map_or(before, ty), "{species:?}");
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}

#[test]
fn sheer_force_and_fainted_users_skip_the_hazard_and_spin_effects() {
    use crate::state::scratch::move_runtime::HAS_SHEER_FORCE;
    // AfterHit (target, source, move) vs AfterSubDamage (damage, target, source, move).
    let after_hit = [mon(6), mon(0), EventArg::Move(0)];
    let after_sub = [EventArg::Number(10.0), mon(6), mon(0), EventArg::Move(0)];
    for hook in [
        dex::HOOK_MOVE_CEASELESSEDGE_ONAFTERHIT,
        dex::HOOK_MOVE_STONEAXE_ONAFTERHIT,
        dex::HOOK_MOVE_RAPIDSPIN_ONAFTERHIT,
        dex::HOOK_MOVE_MORTALSPIN_ONAFTERHIT,
    ] {
        let mut b = battle(NoLog);
        b.scratch.moves[0].as_mut().unwrap().runtime_flags = HAS_SHEER_FORCE;
        assert_eq!(call(&mut b, hook, &after_hit), Relay::Undefined);
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
    for hook in [
        dex::HOOK_MOVE_CEASELESSEDGE_ONAFTERSUBDAMAGE,
        dex::HOOK_MOVE_STONEAXE_ONAFTERSUBDAMAGE,
        dex::HOOK_MOVE_RAPIDSPIN_ONAFTERSUBDAMAGE,
        dex::HOOK_MOVE_MORTALSPIN_ONAFTERSUBDAMAGE,
        dex::HOOK_MOVE_ICESPINNER_ONAFTERSUBDAMAGE,
    ] {
        // A fainted source stops every one of them before any core mutator runs.
        let mut b = battle(NoLog);
        b.state.pokemon[0].hp = 0;
        assert_eq!(call(&mut b, hook, &after_sub), Relay::Undefined);
        // Sheer Force stops all but Ice Spinner, which never reads it.
        if hook != dex::HOOK_MOVE_ICESPINNER_ONAFTERSUBDAMAGE {
            let mut b = battle(NoLog);
            b.scratch.moves[0].as_mut().unwrap().runtime_flags = HAS_SHEER_FORCE;
            assert_eq!(call(&mut b, hook, &after_sub), Relay::Undefined);
            assert_eq!(b.seed(), [1, 2, 3, 4]);
        }
    }
}

#[test]
fn aurora_veil_try_needs_snow() {
    let try_hook = |b: &mut Battle<NoLog>| {
        call(
            b,
            dex::HOOK_MOVE_AURORAVEIL_ONTRY,
            &[mon(0), mon(6), EventArg::Move(0)],
        )
    };
    let mut b = battle(NoLog);
    // No weather: false, which the move pipeline reports as `-fail`.
    assert_eq!(try_hook(&mut b), Relay::Bool(false));
    for (weather, expected) in [
        (dex::CONDITION_SNOWSCAPE, true),
        (dex::CONDITION_RAINDANCE, false),
    ] {
        let cell = b.state.field.weather;
        b.state.effects.cells[cell.0 as usize].id = weather;
        assert_eq!(try_hook(&mut b), Relay::Bool(expected));
    }
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}

// ---------------------------------------------------------------------------------------------
// Entry hazards against the real mutators (needs the unfinished core)
// ---------------------------------------------------------------------------------------------

/// Switch-in order of the hazards on p2's side in scenario `hazards_layers_and_switchins`:
/// Spikes (3 layers), Toxic Spikes (2), Stealth Rock, Sticky Web. HP checkpoints are the exact values
/// the pinned Showdown logged for those switch-ins (`|-damage|..|118/157|[from] Spikes` etc.).
#[test]
#[ignore = "needs core"]
fn entry_hazards_hurt_each_switch_in_like_showdown() {
    use crate::actions::Attribution;
    let sc = parse_scenarios(include_str!("scenarios.txt"))
        .into_iter()
        .find(|s| s.name == "hazards_layers_and_switchins")
        .unwrap();
    // (p2 team index, max HP, HP after Spikes, HP after Stealth Rock, Toxic Spikes absorbed,
    //  Sticky Web applies)
    let table = [
        (0usize, 167u16, 167u16, 126u16, false, false), // Pelipper: Flying
        (1, 167, 126, 106, true, true),                 // Gengar: Poison absorbs Toxic Spikes
        (2, 0, 0, 0, false, false),                     // Garchomp: Heavy-Duty Boots
        (3, 157, 118, 99, true, true),                  // Toxapex: Poison absorbs Toxic Spikes
        (4, 177, 133, 111, false, true),                // Scizor: Steel, Toxic Spikes stay
        (5, 196, 196, 172, false, false),               // Landorus-Therian: Flying
    ];
    for (idx, max_hp, after_spikes, after_rock, absorbs, web) in table {
        let mut b = Battle::with_log(sc.seed, sc.teams[0], sc.teams[1], Trace::default()).unwrap();
        let victim = MonId(6 + idx as u8);
        b.state.sides[0].active = [MonId(0), MonId::NONE];
        b.state.sides[1].active = [victim, MonId::NONE];
        for m in [MonId(0), victim] {
            b.state.pokemon[m.0 as usize].flags |= mon_flags::ACTIVE;
        }
        let attribution = Attribution {
            source: mon(0),
            effect: EffectRef::None,
        };
        for (id, times) in [
            (dex::CONDITION_SPIKES, 3),
            (dex::CONDITION_TOXICSPIKES, 2),
            (dex::CONDITION_STEALTHROCK, 1),
            (dex::CONDITION_STICKYWEB, 1),
        ] {
            for _ in 0..times {
                b.add_side_condition(SideId(1), id, attribution);
            }
        }
        // Fourth Spikes layer / third Toxic Spikes layer are refused.
        assert_eq!(
            b.add_side_condition(SideId(1), dex::CONDITION_SPIKES, attribution),
            Relay::Bool(false)
        );
        let hp = |b: &Battle<Trace>| b.state.pokemon[victim.0 as usize].hp;
        let max = hp(&b);
        if max_hp != 0 {
            assert_eq!(max, max_hp, "fixture HP for team slot {idx}");
        } else {
            b.state.pokemon[victim.0 as usize].item = dex::ITEM_HEAVYDUTYBOOTS;
        }
        let switch_in = |b: &mut Battle<Trace>, id: EffectId| {
            let cell = b.get_side_condition_data(SideId(1), id);
            let state = cell.map(|c| b.state.effects.capture(c));
            b.single_event(
                EventId::SwitchIn,
                EffectRef::Dex(id),
                state,
                EventArg::Holder(Holder::mon(victim)),
                EventArg::Null,
                EffectRef::None,
                Relay::Undefined,
                None,
            );
        };
        switch_in(&mut b, dex::CONDITION_SPIKES);
        assert_eq!(hp(&b), if max_hp == 0 { max } else { after_spikes }, "Spikes");
        switch_in(&mut b, dex::CONDITION_TOXICSPIKES);
        assert_eq!(
            b.get_side_condition(SideId(1), dex::CONDITION_TOXICSPIKES)
                .is_none(),
            absorbs
        );
        assert_eq!(
            b.state.pokemon[victim.0 as usize].status,
            crate::state::Status::None
        );
        switch_in(&mut b, dex::CONDITION_STEALTHROCK);
        assert_eq!(hp(&b), if max_hp == 0 { max } else { after_rock }, "Stealth Rock");
        switch_in(&mut b, dex::CONDITION_STICKYWEB);
        assert_eq!(
            b.state.pokemon[victim.0 as usize].boosts[4],
            if web { -1 } else { 0 },
            "Sticky Web"
        );
        assert_eq!(b.seed(), sc.seed);
    }
}

// ---------------------------------------------------------------------------------------------
// Dispatcher wiring: the same entry points Showdown uses (singleEvent/runEvent, addSideCondition,
// addPseudoWeather) with the TS argument order, against the real mutators (needs the unfinished core)
// ---------------------------------------------------------------------------------------------

fn move_frame(b: &mut Battle<Trace>, id: EffectId) -> u8 {
    let h = b.get_active_move(MoveInput::Dex(id));
    b.set_active_move(Some(h), Some(MonId(0)), Some(MonId(6)));
    h.0
}
/// singleEvent(event, move, {}, target, source, move[, relay]) as battle-actions.ts calls it.
fn single(b: &mut Battle<Trace>, event: EventId, h: u8, target: u8, source: u8, relay: Relay) -> Relay {
    b.single_event(
        event,
        EffectRef::ActiveMove(h),
        None,
        mon(target),
        mon(source),
        EffectRef::ActiveMove(h),
        relay,
        None,
    )
}
fn add_side(b: &mut Battle<Trace>, side: u8, id: EffectId) -> Relay {
    b.add_side_condition(
        SideId(side),
        id,
        Attribution {
            source: mon(0),
            effect: EffectRef::None,
        },
    )
}
fn has_side(b: &Battle<Trace>, side: u8, id: EffectId) -> bool {
    b.get_side_condition(SideId(side), id).is_some()
}
use crate::actions::{Attribution, MoveInput};

#[test]
#[ignore = "needs core"]
fn ceaseless_edge_and_stone_axe_install_foe_hazards_through_the_dispatcher() {
    use crate::state::scratch::move_runtime::HAS_SHEER_FORCE;
    for (mv, hazard, text) in [
        (dex::MOVE_CEASELESSEDGE, dex::CONDITION_SPIKES, "Spikes"),
        (dex::MOVE_STONEAXE, dex::CONDITION_STEALTHROCK, "move: Stealth Rock"),
    ] {
        for (event, relay) in [
            (EventId::AfterHit, Relay::Undefined),
            (EventId::AfterSubDamage, Relay::Number(12.0)),
        ] {
            let mut b = battle(Trace::default());
            let h = move_frame(&mut b, mv);
            single(&mut b, event, h, 6, 0, relay);
            assert!(has_side(&b, 1, hazard) && !has_side(&b, 0, hazard));
            assert_eq!(b.log.0, vec![format!("-sidestart|side1|{text}")]);
            // side.addSideCondition(id) passes no source: it takes the event target (side.ts:414).
            let cell = b.get_side_condition_data(SideId(1), hazard).unwrap();
            assert_eq!(b.state.effects.cells[cell.0 as usize].source, MonId(6));
            assert_eq!(b.seed(), [1, 2, 3, 4]);

            // Sheer Force removed the effect.
            let mut b = battle(Trace::default());
            let h = move_frame(&mut b, mv);
            b.active_move_mut(MoveHandle(h)).runtime_flags |= HAS_SHEER_FORCE;
            single(&mut b, event, h, 6, 0, relay);
            assert!(!has_side(&b, 1, hazard) && b.log.0.is_empty());

            // Behind a Substitute the user must still be alive.
            if event == EventId::AfterSubDamage {
                let mut b = battle(Trace::default());
                let h = move_frame(&mut b, mv);
                b.state.pokemon[0].hp = 0;
                single(&mut b, event, h, 6, 0, relay);
                assert!(!has_side(&b, 1, hazard) && b.log.0.is_empty());
            }
        }
    }
}

#[test]
#[ignore = "needs core"]
fn spins_clear_the_users_side_in_source_order() {
    use crate::state::scratch::move_runtime::HAS_SHEER_FORCE;
    for (mv, name) in [(dex::MOVE_RAPIDSPIN, "Rapid Spin"), (dex::MOVE_MORTALSPIN, "Mortal Spin")] {
        for (event, relay) in [
            (EventId::AfterHit, Relay::Undefined),
            (EventId::AfterSubDamage, Relay::Number(12.0)),
        ] {
            let setup = |b: &mut Battle<Trace>| {
                for id in [
                    dex::CONDITION_STICKYWEB,
                    dex::CONDITION_STEALTHROCK,
                    dex::CONDITION_TOXICSPIKES,
                    dex::CONDITION_SPIKES,
                    dex::CONDITION_REFLECT,
                ] {
                    add_side(b, 0, id);
                }
                // Hazards on the foe's side stay.
                add_side(b, 1, dex::CONDITION_SPIKES);
                let seed = b.state.effects.alloc(
                    Holder::mon(MonId(0)),
                    Holder::mon(MonId(0)),
                    dex::CONDITION_LEECHSEED,
                    0,
                );
                b.state.pokemon[0].volatiles.push(seed);
                b.log.0.clear();
            };
            let mut b = battle(Trace::default());
            setup(&mut b);
            let h = move_frame(&mut b, mv);
            single(&mut b, event, h, 6, 0, relay);
            // Leech Seed first, then spikes/toxicspikes/stealthrock/stickyweb (not the insertion order
            // of the side's list), each tagged with the move and the user.
            let tags = format!("[from] {name}|[of] mon0");
            assert_eq!(
                b.log.0,
                vec![
                    format!("-end|mon0|Leech Seed|{tags}"),
                    format!("-sideend|side0|Spikes|{tags}"),
                    format!("-sideend|side0|Toxic Spikes|{tags}"),
                    format!("-sideend|side0|Stealth Rock|{tags}"),
                    format!("-sideend|side0|Sticky Web|{tags}"),
                ]
            );
            assert!(has_side(&b, 0, dex::CONDITION_REFLECT), "screens are not hazards");
            assert!(has_side(&b, 1, dex::CONDITION_SPIKES));
            assert!(b.get_volatile(MonId(0), dex::CONDITION_LEECHSEED).is_none());
            assert_eq!(b.seed(), [1, 2, 3, 4]);

            // Sheer Force keeps everything; a fainted user (after a Substitute) stops at once.
            let mut b = battle(Trace::default());
            setup(&mut b);
            let h = move_frame(&mut b, mv);
            b.active_move_mut(MoveHandle(h)).runtime_flags |= HAS_SHEER_FORCE;
            single(&mut b, event, h, 6, 0, relay);
            assert!(b.log.0.is_empty() && has_side(&b, 0, dex::CONDITION_SPIKES));
            if event == EventId::AfterSubDamage {
                let mut b = battle(Trace::default());
                setup(&mut b);
                let h = move_frame(&mut b, mv);
                b.state.pokemon[0].hp = 0;
                single(&mut b, event, h, 6, 0, relay);
                assert!(b.log.0.is_empty() && has_side(&b, 0, dex::CONDITION_SPIKES));
            }
        }
    }
}

#[test]
#[ignore = "needs core"]
fn screen_shatterers_remove_reflect_light_screen_aurora_veil_from_the_target_side() {
    for (mv, event_hook_move) in [
        (dex::MOVE_BRICKBREAK, "brickbreak"),
        (dex::MOVE_PSYCHICFANGS, "psychicfangs"),
        (dex::MOVE_RAGINGBULL, "ragingbull"),
    ] {
        let mut b = battle(Trace::default());
        for id in [
            dex::CONDITION_AURORAVEIL,
            dex::CONDITION_REFLECT,
            dex::CONDITION_LIGHTSCREEN,
            dex::CONDITION_SPIKES,
        ] {
            add_side(&mut b, 1, id);
        }
        add_side(&mut b, 0, dex::CONDITION_REFLECT);
        b.log.0.clear();
        let h = move_frame(&mut b, mv);
        // singleEvent('TryHit', moveData, {}, target, source, move): the foe (mon 6) is the target.
        let r = single(&mut b, EventId::TryHit, h, 6, 0, Relay::Undefined);
        assert_eq!(r, Relay::Bool(true), "{event_hook_move}");
        assert_eq!(
            b.log.0,
            vec![
                "-sideend|side1|Reflect",
                "-sideend|side1|move: Light Screen",
                "-sideend|side1|move: Aurora Veil"
            ],
            "{event_hook_move}"
        );
        assert!(has_side(&b, 1, dex::CONDITION_SPIKES), "hazards stay");
        assert!(has_side(&b, 0, dex::CONDITION_REFLECT), "own side untouched");
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}

#[test]
#[ignore = "needs core"]
fn court_change_swaps_through_hit_field() {
    let mut b = battle(Trace::default());
    let h = move_frame(&mut b, dex::MOVE_COURTCHANGE);
    // Nothing to swap: `false` (the move fails), no lines.
    assert_eq!(
        single(&mut b, EventId::HitField, h, 0, 0, Relay::Undefined),
        Relay::Bool(false)
    );
    assert!(b.log.0.is_empty());
    add_side(&mut b, 0, dex::CONDITION_REFLECT);
    add_side(&mut b, 0, dex::CONDITION_WIDEGUARD);
    add_side(&mut b, 1, dex::CONDITION_SPIKES);
    b.log.0.clear();
    assert_eq!(
        single(&mut b, EventId::HitField, h, 0, 0, Relay::Undefined),
        Relay::Bool(true)
    );
    assert_eq!(b.log.0, vec!["-swapsideconditions", "-activate|mon0|move: Court Change"]);
    assert!(has_side(&b, 1, dex::CONDITION_REFLECT) && has_side(&b, 0, dex::CONDITION_SPIKES));
    assert!(has_side(&b, 0, dex::CONDITION_WIDEGUARD), "Wide Guard is not swapped");
    assert!(!has_side(&b, 0, dex::CONDITION_REFLECT));
    // Swapped states announce their end on the side they now live on.
    b.log.0.clear();
    b.remove_side_condition(SideId(1), dex::CONDITION_REFLECT);
    assert_eq!(b.log.0, vec!["-sideend|side1|Reflect"]);
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}

#[test]
#[ignore = "needs core"]
fn durations_come_from_the_direct_callbacks_installed_by_the_mutators() {
    let attribution = Attribution {
        source: mon(0),
        effect: EffectRef::None,
    };
    for (clay, expected) in [(true, 8), (false, 5)] {
        for id in [
            dex::CONDITION_REFLECT,
            dex::CONDITION_LIGHTSCREEN,
            dex::CONDITION_AURORAVEIL,
        ] {
            let mut b = battle(Trace::default());
            if clay {
                b.state.pokemon[0].item = dex::ITEM_LIGHTCLAY;
            }
            b.add_side_condition(SideId(0), id, attribution);
            let cell = b.get_side_condition_data(SideId(0), id).unwrap();
            assert_eq!(b.state.effects.cells[cell.0 as usize].duration, expected, "{id:?}");
        }
    }
    let mut b = battle(Trace::default());
    b.add_side_condition(SideId(0), dex::CONDITION_TAILWIND, attribution);
    let cell = b.get_side_condition_data(SideId(0), dex::CONDITION_TAILWIND).unwrap();
    assert_eq!(b.state.effects.cells[cell.0 as usize].duration, 4);

    // Trick Room: fieldstart line, 5 turns, and the second use removes it again.
    b.log.0.clear();
    assert_eq!(
        b.add_pseudo_weather(dex::CONDITION_TRICKROOM, attribution),
        Relay::Bool(true)
    );
    let cell = b.get_pseudo_weather(dex::CONDITION_TRICKROOM).unwrap();
    assert_eq!(b.state.effects.cells[cell.0 as usize].duration, 5);
    assert_eq!(b.log.0, vec!["-fieldstart|move: Trick Room|[of] mon0"]);
    b.log.0.clear();
    assert_eq!(
        b.add_pseudo_weather(dex::CONDITION_TRICKROOM, attribution),
        Relay::Bool(true)
    );
    assert!(b.get_pseudo_weather(dex::CONDITION_TRICKROOM).is_none());
    assert_eq!(b.log.0, vec!["-fieldend|move: Trick Room"]);
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}

#[test]
#[ignore = "needs core"]
fn tailwind_and_screens_modify_the_event_relay() {
    use crate::event::RunEventOptions;
    let mut b = battle(Trace::default());
    add_side(&mut b, 0, dex::CONDITION_TAILWIND);
    let speed = b.run_event(
        EventId::ModifySpe,
        mon(0),
        EventArg::Null,
        EffectRef::None,
        Relay::Number(100.0),
        RunEventOptions::default(),
    );
    assert_eq!(speed, Relay::Number(200.0));
    // The foe is unaffected.
    let speed = b.run_event(
        EventId::ModifySpe,
        mon(6),
        EventArg::Null,
        EffectRef::None,
        Relay::Number(100.0),
        RunEventOptions::default(),
    );
    assert_eq!(speed, Relay::Number(100.0));

    // runEvent('ModifyDamage', attacker, defender, move, damage): 100 -> 67 (2732/4096) behind the
    // matching screen only, and not at all for a critical hit.
    for (screen, mv, weakened) in [
        (dex::CONDITION_REFLECT, dex::MOVE_BRICKBREAK, true),
        (dex::CONDITION_REFLECT, dex::MOVE_THUNDERBOLT, false),
        (dex::CONDITION_LIGHTSCREEN, dex::MOVE_THUNDERBOLT, true),
        (dex::CONDITION_LIGHTSCREEN, dex::MOVE_BRICKBREAK, false),
        (dex::CONDITION_AURORAVEIL, dex::MOVE_BRICKBREAK, true),
        (dex::CONDITION_AURORAVEIL, dex::MOVE_THUNDERBOLT, true),
    ] {
        for crit in [false, true] {
            let mut b = battle(Trace::default());
            add_side(&mut b, 0, screen);
            let h = b.get_active_move(MoveInput::Dex(mv));
            b.move_hit_data(MonId(0), h).crit = crit;
            let damage = b.run_event(
                EventId::ModifyDamage,
                mon(6),
                mon(0),
                EffectRef::ActiveMove(h.0),
                Relay::Number(100.0),
                RunEventOptions::default(),
            );
            let want = if weakened && !crit { 67.0 } else { 100.0 };
            assert_eq!(damage, Relay::Number(want), "{screen:?} {mv:?} crit={crit}");
        }
    }
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}

#[test]
#[ignore = "needs core"]
fn move_callbacks_see_the_documented_argument_layouts() {
    // Aurora Veil onTry: singleEvent('Try', move, null, pokemon, target, move).
    let mut b = battle(Trace::default());
    let h = move_frame(&mut b, dex::MOVE_AURORAVEIL);
    assert_eq!(single(&mut b, EventId::Try, h, 0, 6, Relay::Undefined), Relay::Bool(false));
    let cell = b.state.field.weather;
    b.state.effects.cells[cell.0 as usize].id = dex::CONDITION_SNOWSCAPE;
    assert_eq!(single(&mut b, EventId::Try, h, 0, 6, Relay::Undefined), Relay::Bool(true));

    // Raging Bull onModifyType: singleEvent('ModifyType', move, null, pokemon, target, move, move).
    let mut b = battle(Trace::default());
    b.state.pokemon[0].species = dex::SPECIES_TAUROSPALDEABLAZE;
    let h = move_frame(&mut b, dex::MOVE_RAGINGBULL);
    single(&mut b, EventId::ModifyType, h, 0, 6, Relay::ActiveMove(h));
    assert_eq!(b.active_move(MoveHandle(h)).move_type, dex::type_id("Fire").unwrap());

    // Tidy Up onHit: clears both sides' hazards, logs, boosts +1 Atk / +1 Spe, returns true.
    let mut b = battle(Trace::default());
    for id in [dex::CONDITION_SPIKES, dex::CONDITION_STEALTHROCK] {
        add_side(&mut b, 0, id);
    }
    add_side(&mut b, 1, dex::CONDITION_STICKYWEB);
    add_side(&mut b, 1, dex::CONDITION_REFLECT);
    b.log.0.clear();
    let h = move_frame(&mut b, dex::MOVE_TIDYUP);
    assert_eq!(single(&mut b, EventId::Hit, h, 0, 0, Relay::Undefined), Relay::Bool(true));
    assert_eq!(b.log.0[..3], ["-sideend|side0|Spikes", "-sideend|side0|Stealth Rock", "-sideend|side1|Sticky Web"]);
    assert_eq!(b.log.0[3], "-activate|mon0|move: Tidy Up");
    assert!(has_side(&b, 1, dex::CONDITION_REFLECT));
    assert_eq!((b.state.pokemon[0].boosts[0], b.state.pokemon[0].boosts[4]), (1, 1));
    // With nothing to clear it still succeeds by boosting; at +6/+6 it fails.
    b.state.pokemon[0].boosts = [6, 0, 0, 0, 6, 0, 0];
    b.log.0.clear();
    assert_eq!(single(&mut b, EventId::Hit, h, 0, 0, Relay::Undefined), Relay::Bool(false));
    assert!(b.log.0.is_empty());

    // Ice Spinner onAfterHit clears the terrain.
    let mut b = battle(Trace::default());
    let cell = b.state.field.terrain;
    b.state.effects.cells[cell.0 as usize].id = dex::CONDITION_GRASSYTERRAIN;
    let h = move_frame(&mut b, dex::MOVE_ICESPINNER);
    single(&mut b, EventId::AfterHit, h, 6, 0, Relay::Undefined);
    assert_eq!(b.state.effects.cells[cell.0 as usize].id, EffectId::NONE);
}
