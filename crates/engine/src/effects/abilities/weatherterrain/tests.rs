//! Tests for the weather_terrain batch.
//!
//! * `callbacks.tsv` holds oracle rows from tools/probes/weather_terrain/callbacks.mjs: the
//!   pinned Showdown handlers run against stub contexts. Rows that do not consult
//!   `Field::isTerrain`, and the terrain rows, replay against the real Rust core queries.
//! * `scenarios.txt` holds whole recorded battles (log, turn and PRNG seed at every decision
//!   boundary) from tools/probes/weather_terrain/scenarios.mjs. Rain awaits Roost; snow awaits Protect.
use super::*;
use crate::{
    Battle,
    dex::{self, Category, MoveTarget},
    event::{CallArgs, EffectRef, EventArg, EventFrame, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, LogView, MoveLineEdit, NoLog, TextLog},
    state::{
        mon_flags,
        scratch::{ActiveMove, HitData, MoveAccuracy, MoveEffectsScratch, OrderedBoosts},
    },
    teams::TeamDef,
};

const CALLBACKS: &str = include_str!("callbacks.tsv");
const SCENARIOS: &str = include_str!("scenarios.txt");

// ---------------------------------------------------------------------------------
// Callback-level oracle rows.
// ---------------------------------------------------------------------------------
fn effects(base: &'static dex::MoveEffects) -> MoveEffectsScratch {
    MoveEffectsScratch {
        base,
        boosts: OrderedBoosts::default(),
        status: base.status,
        volatile_status: base.volatile_status,
        side_condition: base.side_condition,
        slot_condition: base.slot_condition,
        weather: base.weather,
        terrain: base.terrain,
        pseudo_weather: base.pseudo_weather,
        chance: None,
        heal: base.heal,
        force_switch: base.force_switch,
        self_switch: base.self_switch,
        suppressed_hooks: 0,
    }
}
fn active_move(id: EffectId) -> ActiveMove {
    let m = dex::move_data(id);
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
        base_power: m.base_power as f64,
        accuracy: MoveAccuracy::Percent(100.0),
        hit_data: [HitData::default(); 12],
        self_boosts: OrderedBoosts::default(),
        effects: effects(&m.effects),
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
        category: Category::Special,
        target: m.target,
        priority: m.priority,
        crit_ratio: m.crit_ratio,
        hit: 0,
        multihit: m.multihit,
    }
}
const HOLDER: MonId = MonId(0);
const FOE: MonId = MonId(6);
fn battle() -> Battle<NoLog> {
    battle_with(NoLog)
}
fn battle_with<L: LogSink>(log: L) -> Battle<L> {
    let packed = "Pikachu|||static|thunderbolt|Serious||M|||100|,,,,,Electric";
    let mut b = Battle::with_log([1, 2, 3, 4], packed, packed, log).unwrap();
    for (side, mon) in [(0, HOLDER), (1, FOE)] {
        b.state.sides[side].active[0] = mon;
        b.state.pokemon[mon.0 as usize].flags |= mon_flags::ACTIVE;
        b.state.pokemon[mon.0 as usize].hp = 300;
        b.state.pokemon[mon.0 as usize].max_hp = 300;
    }
    b.scratch.frames[0] = Some(EventFrame {
        event: EventId::BasePower,
        target: EventArg::Holder(Holder::mon(HOLDER)),
        source: EventArg::Holder(Holder::mon(FOE)),
        source_effect: EffectRef::None,
        relay: Relay::Undefined,
        modifier: 4096,
        modifier_present: true,
        has_relay: true,
        fast_exit: false,
        args: [EventArg::Undefined; 4],
        arg_count: 0,
    });
    b.scratch.current_frame = 0;
    // The stub frame is event depth 0; nested events (Type queries) must use frame 1.
    b.scratch.event_depth = 1;
    b
}
fn ty(name: &str) -> TypeId {
    dex::type_id(name).unwrap_or_else(|| panic!("type {name}"))
}
fn set_types<L: LogSink>(b: &mut Battle<L>, mon: MonId, names: &str) {
    let mut types = [TypeId::NONE; 2];
    for (i, n) in names.split(',').enumerate() {
        types[i] = ty(n);
    }
    let p = &mut b.state.pokemon[mon.0 as usize];
    p.types = types;
    p.apparent_types = types;
}
fn add_semi_invulnerable<L: LogSink>(b: &mut Battle<L>, mon: MonId) {
    let cell = b.state.effects.alloc(
        Holder::mon(mon),
        Holder::mon(mon),
        dex::CONDITION_PHANTOMFORCE,
        0,
    );
    b.state.pokemon[mon.0 as usize].volatiles.push(cell);
}
fn condition(name: &str) -> EffectId {
    if name.is_empty() {
        EffectId::NONE
    } else {
        dex::lookup(EffectKind::Condition, name).unwrap_or_else(|| panic!("condition {name}"))
    }
}
fn effect_of(reference: &str) -> EffectId {
    let (kind, name) = reference.split_once(':').unwrap();
    let kind = match kind {
        "abilities" => EffectKind::Ability,
        "moves" => EffectKind::Move,
        _ => EffectKind::Condition, // "conditions" and the move-embedded "terrain" conditions
    };
    dex::lookup(kind, name).unwrap_or_else(|| panic!("effect {reference}"))
}
fn hook_of(id: EffectId, key: &str) -> dex::HookId {
    let m = &dex::MANIFESTS[id.0 as usize];
    let i = m
        .hooks()
        .iter()
        .position(|h| h.key == key && h.site.is_empty())
        .unwrap_or_else(|| panic!("{} has no {key} hook", dex::effect(id).key));
    dex::HookId(m.hooks_start + i as u16)
}
fn immunity(name: &str) -> dex::ImmunityId {
    match name {
        "sandstorm" => dex::ImmunityId::Sandstorm,
        "hail" => dex::ImmunityId::Hail,
        "frz" => dex::ImmunityId::Frz,
        "par" => dex::ImmunityId::Par,
        _ => panic!("immunity {name}"),
    }
}
/// Runs one oracle row; returns false for rows that need the terrain core (usesTerrain).
fn run_row(line: &str, terrain_rows: bool) -> bool {
    let c: Vec<&str> = line.split('\t').collect();
    assert_eq!(c.len(), 20, "{line}");
    if (c[19] == "1") != terrain_rows {
        return false;
    }
    let mut b = battle();
    let id = effect_of(c[0]);
    let hook = hook_of(id, c[1]);
    let w = b.state.field.weather.0 as usize;
    b.state.effects.cells[w].id = condition(c[2]);
    let t = b.state.field.terrain.0 as usize;
    b.state.effects.cells[t].id = condition(c[3]);
    set_types(&mut b, HOLDER, c[4]);
    set_types(&mut b, FOE, c[6]);
    if c[5] == "1" {
        add_semi_invulnerable(&mut b, HOLDER);
    }
    if c[7] == "1" {
        add_semi_invulnerable(&mut b, FOE);
    }
    let mut mv = active_move(dex::lookup(EffectKind::Move, c[9]).unwrap());
    mv.move_type = ty(c[8]);
    mv.accuracy = MoveAccuracy::Percent(70.0);
    mv.base_power = 50.0;
    mv.target = MoveTarget::Normal;
    mv.priority = 0;
    b.scratch.moves[0] = Some(mv);
    let input = c[10].parse::<f64>().unwrap_or(0.0);
    let holder = EventArg::Holder(Holder::mon(HOLDER));
    let foe = if c[12] == "1" {
        EventArg::Undefined
    } else {
        EventArg::Holder(Holder::mon(FOE))
    };
    let move_effect = EventArg::Effect(EffectRef::ActiveMove(0));
    let values = match c[1] {
        "onModifyMove" | "onModifyType" => [EventArg::Move(0), holder, foe, move_effect],
        "onModifySpe" | "onModifySpD" | "onModifyDef" => [
            EventArg::Number(input),
            holder,
            EventArg::Null,
            EventArg::Undefined,
        ],
        "onModifyPriority" => [EventArg::Number(input), holder, foe, EventArg::Undefined],
        "onImmunity" => [
            EventArg::Relay(Relay::StatusImmunity(immunity(c[11]))),
            holder,
            EventArg::Null,
            EventArg::Undefined,
        ],
        _ => [EventArg::Number(input), holder, foe, move_effect],
    };
    let relay = b.call_hook(hook, CallArgs { values, len: 4 });
    let expected_relay = match c[13] {
        "undefined" => Relay::Undefined,
        "false" => Relay::Bool(false),
        n => Relay::Number(n.parse().unwrap()),
    };
    assert_eq!(relay, expected_relay, "relay: {line}");
    assert_eq!(
        b.scratch.frames[0].unwrap().modifier,
        c[14].parse::<u32>().unwrap(),
        "modifier: {line}"
    );
    let mv = b.scratch.moves[0].unwrap();
    let accuracy = match c[15] {
        "true" => MoveAccuracy::Always,
        n => MoveAccuracy::Percent(n.parse().unwrap()),
    };
    assert_eq!(mv.accuracy, accuracy, "accuracy: {line}");
    assert_eq!(mv.base_power, c[16].parse::<f64>().unwrap(), "bp: {line}");
    assert_eq!(mv.move_type, ty(c[17]), "type: {line}");
    let target = match c[18] {
        "normal" => MoveTarget::Normal,
        "allAdjacentFoes" => MoveTarget::AllAdjacentFoes,
        other => panic!("target {other}"),
    };
    assert_eq!(mv.target, target, "target: {line}");
    assert_eq!(b.seed(), [1, 2, 3, 4], "handlers draw nothing: {line}");
    true
}
fn rows() -> impl Iterator<Item = &'static str> {
    CALLBACKS.lines().filter(|l| !l.starts_with('#'))
}

#[test]
fn weather_and_terrain_callbacks_match_pinned_showdown() {
    let n = rows().filter(|l| run_row(l, false)).count();
    assert!(n > 700, "{n} rows");
}
#[test]
fn terrain_dependent_callbacks_match_pinned_showdown() {
    let n = rows().filter(|l| run_row(l, true)).count();
    assert!(n > 40, "{n} rows");
}
#[test]
fn type_constants_match_the_dex_names() {
    for (constant, name) in [
        (TYPE_ELECTRIC, "Electric"),
        (TYPE_FIRE, "Fire"),
        (TYPE_GRASS, "Grass"),
        (TYPE_GROUND, "Ground"),
        (TYPE_ICE, "Ice"),
        (TYPE_PSYCHIC, "Psychic"),
        (TYPE_ROCK, "Rock"),
        (TYPE_STEEL, "Steel"),
        (TYPE_WATER, "Water"),
    ] {
        assert_eq!(Some(constant), dex::type_id(name), "{name}");
    }
}

// ---------------------------------------------------------------------------------
// Recorded whole-battle scenarios.
// ---------------------------------------------------------------------------------
enum Op<'a> {
    Boundary {
        turn: u32,
        seed: [u16; 4],
        log: Vec<&'a str>,
    },
    Choose {
        side: usize,
        text: &'a str,
    },
}
struct Scenario<'a> {
    name: &'a str,
    seed: [u16; 4],
    teams: [&'a str; 2],
    ops: Vec<Op<'a>>,
}
fn seed(text: &str) -> [u16; 4] {
    let v: Vec<u16> = text.split(',').map(|n| n.parse().unwrap()).collect();
    v.try_into().unwrap()
}
fn scenarios() -> Vec<Scenario<'static>> {
    let mut out: Vec<Scenario> = Vec::new();
    for line in SCENARIOS.lines().filter(|l| !l.starts_with('#')) {
        let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
        if tag == "S" {
            out.push(Scenario {
                name: rest,
                seed: [0; 4],
                teams: [""; 2],
                ops: Vec::new(),
            });
            continue;
        }
        let sc = out.last_mut().expect("scenario header first");
        match tag {
            "SEED" => sc.seed = seed(rest),
            "T1" => sc.teams[0] = rest,
            "T2" => sc.teams[1] = rest,
            "B" => {
                let (turn, s) = rest.split_once(' ').unwrap();
                sc.ops.push(Op::Boundary {
                    turn: turn.parse().unwrap(),
                    seed: seed(s),
                    log: Vec::new(),
                });
            }
            "L" => match sc.ops.last_mut() {
                Some(Op::Boundary { log, .. }) => log.push(rest),
                _ => panic!("log line outside a boundary"),
            },
            "C" => {
                let (side, text) = rest.split_once(' ').unwrap();
                sc.ops.push(Op::Choose {
                    side: usize::from(side == "p2"),
                    text,
                });
            }
            "E" => {}
            other => panic!("unknown scenario tag {other}"),
        }
    }
    out
}
fn normalize(line: &str) -> &str {
    let digits = line.strip_prefix("|t:|");
    if digits.is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())) {
        "|t:|"
    } else {
        line
    }
}
fn replay(name: &str) {
    let all = scenarios();
    let sc = all.iter().find(|s| s.name == name).expect("scenario");
    let mut b = Battle::from_players(
        sc.seed,
        ("Alice", sc.teams[0]),
        ("Bob", sc.teams[1]),
        TextLog::default(),
    )
    .unwrap();
    b.start().unwrap();
    for op in &sc.ops {
        match op {
            Op::Boundary { turn, seed, log } => {
                let mut got = Vec::new();
                b.drain_log(&mut got);
                let got: Vec<&str> = got.iter().map(|l| normalize(l)).collect();
                let first = got.iter().zip(log).position(|(a, e)| a != e);
                assert!(
                    first.is_none() && got.len() == log.len(),
                    "{name} before turn {turn}: first differing line {first:?} (got {} lines, want {}): got {:?}, want {:?}",
                    got.len(),
                    log.len(),
                    first.map(|i| got[i]),
                    first.map(|i| log[i]),
                );
                assert_eq!(b.seed(), *seed, "{name}: PRNG seed at turn {turn}");
                assert_eq!(b.turn(), *turn, "{name}: turn counter");
            }
            Op::Choose { side, text } => b
                .choose(*side, text)
                .unwrap_or_else(|e| panic!("{name}: side {side} choice {text:?}: {e:?}")),
        }
    }
}

#[test]
fn scenario_file_is_well_formed_and_every_team_is_in_scope() {
    let all = scenarios();
    assert_eq!(all.len(), 8);
    for sc in &all {
        for team in sc.teams {
            let t = TeamDef::unpack(team).unwrap_or_else(|e| panic!("{}: {e}", sc.name));
            assert_eq!(t.len, 2);
        }
        Battle::from_players(sc.seed, ("Alice", sc.teams[0]), ("Bob", sc.teams[1]), NoLog)
            .unwrap_or_else(|e| panic!("{}: {e}", sc.name));
        let boundaries = sc
            .ops
            .iter()
            .filter(|o| matches!(o, Op::Boundary { .. }))
            .count();
        assert!(boundaries >= 7, "{}: {boundaries} boundaries", sc.name);
        assert!(matches!(sc.ops.first(), Some(Op::Boundary { turn: 1, .. })));
    }
}
macro_rules! scenario_tests {
    ($($name:ident),*) => {$(
        #[test]
        fn $name() {
            replay(stringify!($name));
        }
    )*};
}
scenario_tests!(
    weather_leads,
    sun,
    sand,
    electric_terrain,
    grassy_terrain,
    psychic_terrain
);

#[test]
#[ignore = "pending move_callbacks: conditions:roost onStart"]
fn rain() { replay("rain"); }

#[test]
#[ignore = "pending protect_redirection: moves:protect onPrepareHit"]
fn snow() { replay("snow"); }

// ---------------------------------------------------------------------------------
// Handler branches that log or mutate state, against a recording sink. Expected lines are
// the pinned source's `this.add(...)` calls (data/conditions.ts, data/moves.ts, data/abilities.ts).
// ---------------------------------------------------------------------------------
#[derive(Default)]
struct Rec(Vec<String>);
fn key(e: EffectRef) -> &'static str {
    match e {
        EffectRef::Dex(id) => dex::effect(id).key,
        other => panic!("unexpected effect {other:?}"),
    }
}
impl LogSink for Rec {
    const ENABLED: bool = true;
    fn emit(&mut self, _: LogView<'_>, entry: LogEntry<'_>) {
        let mut line = String::from(entry.command);
        for a in entry.args {
            line.push('|');
            match a {
                LogArg::Text(t) => line.push_str(t),
                LogArg::Mon(m) => line.push_str(&format!("p{}", m.0)),
                other => panic!("unexpected arg {other:?}"),
            }
        }
        for t in entry.tags {
            line.push('|');
            match t {
                LogTag::From(e) => line.push_str(&format!("[from] {}", key(*e))),
                LogTag::Of(m) => line.push_str(&format!("[of] p{}", m.0)),
                LogTag::Bare(s) => line.push_str(&format!("[{s}]")),
                other => panic!("unexpected tag {other:?}"),
            }
        }
        self.0.push(line);
    }
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {}
}
fn recorder() -> Battle<Rec> {
    battle_with(Rec::default())
}
fn direct<L: LogSink>(b: &mut Battle<L>, id: EffectId, key: &str, values: [EventArg; 4]) -> Relay {
    b.call_hook(hook_of(id, key), CallArgs { values, len: 4 })
}
fn holder(m: MonId) -> EventArg {
    EventArg::Holder(Holder::mon(m))
}
fn set_field_weather<L: LogSink>(b: &mut Battle<L>, id: EffectId) {
    let i = b.state.field.weather.0 as usize;
    b.state.effects.cells[i].id = id;
}
fn move_with_status(b: &mut Battle<Rec>, status: bool, secondaries: bool) {
    let mut mv = active_move(dex::MOVE_THUNDERBOLT);
    mv.effects.status = if status {
        dex::CONDITION_PAR
    } else {
        EffectId::NONE
    };
    mv.secondaries_present = secondaries;
    b.scratch.moves[0] = Some(mv);
}
const NO_ARGS: [EventArg; 4] = [EventArg::Undefined; 4];

#[test]
fn field_start_names_the_ability_source_only_for_ability_effects() {
    for (cond, name) in [
        (dex::CONDITION_RAINDANCE, "RainDance"),
        (dex::CONDITION_SUNNYDAY, "SunnyDay"),
        (dex::CONDITION_SANDSTORM, "Sandstorm"),
        (dex::CONDITION_SNOWSCAPE, "Snowscape"),
    ] {
        let mut b = recorder();
        let start = |effect| {
            [
                EventArg::Holder(Holder::FIELD),
                holder(HOLDER),
                EventArg::Effect(effect),
                EventArg::Undefined,
            ]
        };
        direct(
            &mut b,
            cond,
            "onFieldStart",
            start(EffectRef::Dex(dex::ABILITY_DROUGHT)),
        );
        direct(
            &mut b,
            cond,
            "onFieldStart",
            start(EffectRef::Dex(dex::MOVE_RAINDANCE)),
        );
        assert_eq!(
            b.log.0,
            [
                format!("-weather|{name}|[from] drought|[of] p0"),
                format!("-weather|{name}"),
            ],
        );
    }
    for (cond, name) in [
        (dex::CONDITION_ELECTRICTERRAIN, "move: Electric Terrain"),
        (dex::CONDITION_GRASSYTERRAIN, "move: Grassy Terrain"),
        (dex::CONDITION_PSYCHICTERRAIN, "move: Psychic Terrain"),
    ] {
        let mut b = recorder();
        let start = |effect| {
            [
                EventArg::Holder(Holder::FIELD),
                holder(FOE),
                EventArg::Effect(effect),
                EventArg::Undefined,
            ]
        };
        direct(
            &mut b,
            cond,
            "onFieldStart",
            start(EffectRef::Dex(dex::ABILITY_SEEDSOWER)),
        );
        direct(
            &mut b,
            cond,
            "onFieldStart",
            start(EffectRef::Dex(dex::MOVE_PSYCHICNOISE)),
        );
        direct(&mut b, cond, "onFieldEnd", NO_ARGS);
        assert_eq!(
            b.log.0,
            [
                format!("-fieldstart|{name}|[from] seedsower|[of] p6"),
                format!("-fieldstart|{name}"),
                format!("-fieldend|{name}"),
            ],
        );
    }
    let mut b = recorder();
    direct(&mut b, dex::CONDITION_SNOWSCAPE, "onFieldEnd", NO_ARGS);
    assert_eq!(b.log.0, ["-weather|none"]);
}
#[test]
fn weather_residual_always_prints_upkeep_before_the_weather_event() {
    for (cond, name) in [
        (dex::CONDITION_RAINDANCE, "RainDance"),
        (dex::CONDITION_SUNNYDAY, "SunnyDay"),
        (dex::CONDITION_SANDSTORM, "Sandstorm"),
        (dex::CONDITION_SNOWSCAPE, "Snowscape"),
    ] {
        // Active weather: the upkeep line, then eachEvent('Weather'). Sandstorm's own onWeather
        // damages both Pikachu through `damage`, which belongs to the unfinished mutators (the
        // `sand` scenario covers it).
        if cond != dex::CONDITION_SANDSTORM {
            let mut b = recorder();
            set_field_weather(&mut b, cond);
            direct(&mut b, cond, "onFieldResidual", NO_ARGS);
            assert_eq!(b.log.0, [format!("-weather|{name}|[upkeep]")]);
        }
        // The guarded weathers skip the event when the weather is no longer theirs (e.g. Air
        // Lock) but still print the upkeep line.
        let mut b = recorder();
        set_field_weather(&mut b, EffectId::NONE);
        direct(&mut b, cond, "onFieldResidual", NO_ARGS);
        assert_eq!(b.log.0, [format!("-weather|{name}|[upkeep]")]);
    }
}
#[test]
fn duration_callbacks_are_five_without_extender_items() {
    let source = [
        holder(HOLDER),
        holder(HOLDER),
        EventArg::Undefined,
        EventArg::Undefined,
    ];
    let mut b = battle();
    for cond in [
        dex::CONDITION_RAINDANCE,
        dex::CONDITION_SUNNYDAY,
        dex::CONDITION_SANDSTORM,
        dex::CONDITION_SNOWSCAPE,
        dex::CONDITION_ELECTRICTERRAIN,
        dex::CONDITION_GRASSYTERRAIN,
        dex::CONDITION_PSYCHICTERRAIN,
    ] {
        assert_eq!(
            direct(&mut b, cond, "durationCallback", source),
            Relay::Number(5.0),
            "{}",
            dex::effect(cond).key
        );
    }
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}
#[test]
fn leaf_guard_blocks_status_in_sun_and_announces_only_for_primary_status_effects() {
    let set_status = |effect: EventArg| {
        [
            EventArg::Relay(Relay::Effect(dex::CONDITION_BRN)),
            holder(HOLDER),
            holder(FOE),
            effect,
        ]
    };
    let by_move = EventArg::Effect(EffectRef::ActiveMove(0));
    // Sun, a move whose primary effect is a status: -immune and false.
    let mut b = recorder();
    set_field_weather(&mut b, dex::CONDITION_SUNNYDAY);
    move_with_status(&mut b, true, false);
    let r = direct(
        &mut b,
        dex::ABILITY_LEAFGUARD,
        "onSetStatus",
        set_status(by_move),
    );
    assert_eq!(r, Relay::Bool(false));
    assert_eq!(b.log.0, ["-immune|p0|[from] leafguard"]);
    // Sun, a secondary-status move (no top-level status): silently false.
    let mut b = recorder();
    set_field_weather(&mut b, dex::CONDITION_SUNNYDAY);
    move_with_status(&mut b, false, true);
    let r = direct(
        &mut b,
        dex::ABILITY_LEAFGUARD,
        "onSetStatus",
        set_status(by_move),
    );
    assert_eq!((r, b.log.0.len()), (Relay::Bool(false), 0));
    // Synchronize's synthetic effect carries a status property.
    let mut b = recorder();
    set_field_weather(&mut b, dex::CONDITION_SUNNYDAY);
    let synchronize = EventArg::Effect(EffectRef::Synchronize(dex::CONDITION_BRN));
    let r = direct(
        &mut b,
        dex::ABILITY_LEAFGUARD,
        "onSetStatus",
        set_status(synchronize),
    );
    assert_eq!((r, b.log.0.len()), (Relay::Bool(false), 1));
    // No sun: no effect.
    let mut b = recorder();
    move_with_status(&mut b, true, false);
    let r = direct(
        &mut b,
        dex::ABILITY_LEAFGUARD,
        "onSetStatus",
        set_status(by_move),
    );
    assert_eq!((r, b.log.0.len()), (Relay::Undefined, 0));
    // Yawn in sun: -immune and null; any other volatile passes.
    let mut b = recorder();
    set_field_weather(&mut b, dex::CONDITION_SUNNYDAY);
    let volatile = |id| {
        [
            EventArg::Relay(Relay::Effect(id)),
            holder(HOLDER),
            holder(FOE),
            EventArg::Undefined,
        ]
    };
    let r = direct(
        &mut b,
        dex::ABILITY_LEAFGUARD,
        "onTryAddVolatile",
        volatile(dex::CONDITION_YAWN),
    );
    assert_eq!(r, Relay::Null);
    let r = direct(
        &mut b,
        dex::ABILITY_LEAFGUARD,
        "onTryAddVolatile",
        volatile(dex::CONDITION_TAUNT),
    );
    assert_eq!(r, Relay::Undefined);
    assert_eq!(b.log.0, ["-immune|p0|[from] leafguard"]);
}
#[test]
fn electric_terrain_sleep_and_yawn_immunity_branches() {
    let status_args = |status: EffectId, effect: EventArg| {
        [
            EventArg::Relay(Relay::Effect(status)),
            holder(HOLDER),
            holder(FOE),
            effect,
        ]
    };
    let by_move = EventArg::Effect(EffectRef::ActiveMove(0));
    let electric = dex::CONDITION_ELECTRICTERRAIN;
    // Sleep from a plain status move on a grounded target: announced.
    let mut b = recorder();
    move_with_status(&mut b, true, false);
    let r = direct(
        &mut b,
        electric,
        "onSetStatus",
        status_args(dex::CONDITION_SLP, by_move),
    );
    assert_eq!(r, Relay::Bool(false));
    assert_eq!(b.log.0, ["-activate|p0|move: Electric Terrain"]);
    // A move with secondaries stays silent (the relay is still false).
    let mut b = recorder();
    move_with_status(&mut b, false, true);
    let r = direct(
        &mut b,
        electric,
        "onSetStatus",
        status_args(dex::CONDITION_SLP, by_move),
    );
    assert_eq!((r, b.log.0.len()), (Relay::Bool(false), 0));
    // Yawn's end: announced through the effect id although it is not a Move effect.
    let mut b = recorder();
    let yawn_effect = EventArg::Effect(EffectRef::Dex(dex::CONDITION_YAWN));
    let r = direct(
        &mut b,
        electric,
        "onSetStatus",
        status_args(dex::CONDITION_SLP, yawn_effect),
    );
    assert_eq!((r, b.log.0.len()), (Relay::Bool(false), 1));
    // Other statuses pass; airborne and semi-invulnerable targets are not protected.
    let mut b = recorder();
    move_with_status(&mut b, true, false);
    let burn = status_args(dex::CONDITION_BRN, by_move);
    assert_eq!(
        direct(&mut b, electric, "onSetStatus", burn),
        Relay::Undefined
    );
    set_types(&mut b, HOLDER, "Flying");
    let sleep = status_args(dex::CONDITION_SLP, by_move);
    assert_eq!(
        direct(&mut b, electric, "onSetStatus", sleep),
        Relay::Undefined
    );
    set_types(&mut b, HOLDER, "Normal");
    add_semi_invulnerable(&mut b, HOLDER);
    assert_eq!(
        direct(&mut b, electric, "onSetStatus", sleep),
        Relay::Undefined
    );
    assert_eq!(b.log.0.len(), 0);
    // Yawn on a grounded target: -activate and null; other volatiles pass.
    let mut b = recorder();
    let yawn = status_args(dex::CONDITION_YAWN, EventArg::Undefined);
    assert_eq!(
        direct(&mut b, electric, "onTryAddVolatile", yawn),
        Relay::Null
    );
    assert_eq!(b.log.0, ["-activate|p0|move: Electric Terrain"]);
    let taunt = status_args(dex::CONDITION_TAUNT, EventArg::Undefined);
    assert_eq!(
        direct(&mut b, electric, "onTryAddVolatile", taunt),
        Relay::Undefined
    );
}
#[test]
fn psychic_terrain_blocks_only_priority_moves_against_grounded_foes() {
    let try_hit = |b: &mut Battle<Rec>, target: MonId| {
        direct(
            b,
            dex::CONDITION_PSYCHICTERRAIN,
            "onTryHit",
            [
                holder(target),
                holder(FOE),
                EventArg::Effect(EffectRef::ActiveMove(0)),
                EventArg::Undefined,
            ],
        )
    };
    let with_move = |b: &mut Battle<Rec>, priority: i8, target: MoveTarget| {
        let mut mv = active_move(dex::MOVE_THUNDERBOLT);
        mv.priority = priority;
        mv.target = target;
        b.scratch.moves[0] = Some(mv);
    };
    // Priority against a grounded foe: -activate, null.
    let mut b = recorder();
    with_move(&mut b, 2, MoveTarget::Normal);
    assert_eq!(try_hit(&mut b, HOLDER), Relay::Null);
    assert_eq!(b.log.0, ["-activate|p0|move: Psychic Terrain"]);
    // Priority 0 and self-targeting moves are never blocked.
    let mut b = recorder();
    with_move(&mut b, 0, MoveTarget::Normal);
    assert_eq!(try_hit(&mut b, HOLDER), Relay::Undefined);
    with_move(&mut b, 1, MoveTarget::SelfTarget);
    assert_eq!(try_hit(&mut b, HOLDER), Relay::Undefined);
    // Semi-invulnerable targets and allies of the attacker are skipped.
    with_move(&mut b, 1, MoveTarget::Normal);
    add_semi_invulnerable(&mut b, HOLDER);
    assert_eq!(try_hit(&mut b, HOLDER), Relay::Undefined);
    assert_eq!(try_hit(&mut b, MonId(7)), Relay::Undefined);
    assert_eq!(b.log.0.len(), 0);
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}
#[test]
fn air_lock_and_cloud_nine_toggle_the_ending_flag_that_suppresses_weather() {
    for (ability, name) in [
        (dex::ABILITY_AIRLOCK, "Air Lock"),
        (dex::ABILITY_CLOUDNINE, "Cloud Nine"),
    ] {
        let mut b = recorder();
        b.state.pokemon[HOLDER.0 as usize].ability = ability;
        set_field_weather(&mut b, dex::CONDITION_RAINDANCE);
        let on = [
            holder(HOLDER),
            EventArg::Undefined,
            EventArg::Undefined,
            EventArg::Undefined,
        ];
        // Switch-in: announce, then onStart clears `ending`; the holder suppresses the weather.
        direct(&mut b, ability, "onSwitchIn", on);
        assert_eq!(b.log.0, [format!("-ability|p0|{name}")]);
        assert_eq!(b.field_effective_weather(), EffectId::NONE);
        // End: `ending` is truthy, so the leaving holder no longer suppresses.
        direct(&mut b, ability, "onEnd", on);
        assert_eq!(b.field_effective_weather(), dex::CONDITION_RAINDANCE);
        // A later onStart (Skill Swap in) clears it again, without a new announcement.
        direct(&mut b, ability, "onStart", on);
        assert_eq!(b.field_effective_weather(), EffectId::NONE);
        assert_eq!(b.log.0.len(), 1);
    }
}
