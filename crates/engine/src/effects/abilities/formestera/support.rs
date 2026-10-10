//! Test-only context adapter. Allocations belong to tests, never battle handlers.
use crate::{
    Battle,
    actions::*,
    dex::{self, Category},
    event::*,
    ids::*,
    log::*,
    state::{mon_flags, scratch::*},
};
#[derive(Default)]
pub(crate) struct Recorder(pub Vec<String>);
fn arg(a: LogArg<'_>) -> String {
    match a {
        LogArg::Text(s) => s.into(),
        LogArg::Empty => String::new(),
        LogArg::Mon(m) => format!("m{}", m.0),
        LogArg::Parts(parts) => parts.iter().map(|&p| arg(p)).collect::<String>(),
        LogArg::Effect(EffectRef::Dex(id)) => dex::effect(id).name.into(),
        LogArg::EffectFullName(EffectRef::Dex(id)) => format!(
            "{}: {}",
            match id.kind().unwrap() {
                EffectKind::Ability => "ability",
                EffectKind::Item => "item",
                EffectKind::Move => "move",
                _ => "condition",
            },
            dex::effect(id).name
        ),
        LogArg::Type(t) => dex::TYPE_NAMES[t.0 as usize - 1].into(),
        LogArg::Number(n) => n.to_string(),
        LogArg::Details(m) => format!("details{}", m.0),
        LogArg::Species(id) => dex::species(id).name.into(),
        LogArg::Health(m) => format!("health{}", m.0),
        _ => panic!("unsupported test log arg {a:?}"),
    }
}
impl LogSink for Recorder {
    const ENABLED: bool = true;
    fn emit(&mut self, _: LogView<'_>, e: LogEntry<'_>) {
        let mut v = vec![e.command.to_string()];
        v.extend(e.args.iter().map(|&a| arg(a)));
        for tag in e.tags {
            v.push(match *tag {
                LogTag::Bare(s) => format!("[{s}]"),
                LogTag::From(x) => format!("[from] {}", arg(LogArg::EffectFullName(x))),
                LogTag::Of(m) => format!("[of] m{}", m.0),
                _ => panic!("test tag {tag:?}"),
            });
        }
        self.0.push(v.join("|"));
    }
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {
        panic!("use battle oracle for move-line edits")
    }
}
pub(crate) fn effects(base: &'static dex::MoveEffects) -> MoveEffectsScratch {
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
pub(crate) fn active_move(id: EffectId) -> ActiveMove {
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
        category: m.category,
        target: m.target,
        priority: m.priority,
        crit_ratio: m.crit_ratio,
        hit: 0,
        multihit: m.multihit,
    }
}

pub(crate) fn battle() -> Battle<Recorder> {
    let packed = "Pikachu|||static|thunderbolt,protect|Serious||M|||100|,,,,,Electric";
    let team = format!("{packed}]{packed}]{packed}]{packed}");
    let mut b = Battle::with_log([1, 2, 3, 4], &team, &team, Recorder::default()).unwrap();
    for side in 0..2 {
        for pos in 0..2 {
            let m = MonId((side * 6 + pos) as u8);
            b.state.sides[side].active[pos] = m;
            b.state.pokemon[m.0 as usize].flags |= mon_flags::ACTIVE;
            b.state.pokemon[m.0 as usize].ability = EffectId::NONE;
        }
    }
    b.scratch.frames[0] = Some(EventFrame {
        event: EventId::BasePower,
        target: holder(0),
        source: holder(6),
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
    b.scratch.event_depth = 1;
    b
}
pub(crate) fn holder(i: u8) -> EventArg {
    EventArg::Holder(Holder::mon(MonId(i)))
}
pub(crate) fn id(kind: EffectKind, s: &str) -> EffectId {
    if s.is_empty() {
        EffectId::NONE
    } else {
        dex::lookup(kind, s).unwrap()
    }
}
pub(crate) fn invoke<L: LogSink>(
    b: &mut Battle<L>,
    id: EffectId,
    key: &str,
    args: [EventArg; 4],
) -> Relay {
    let m = &dex::MANIFESTS[id.0 as usize];
    let Some(i) = m.hooks().iter().position(|h| {
        h.key == key && h.site.is_empty() && matches!(h.value, dex::HookValue::Function)
    }) else {
        return Relay::Undefined;
    };
    b.call_hook(
        dex::HookId(m.hooks_start + i as u16),
        CallArgs {
            values: args,
            len: 4,
        },
    )
}
pub(crate) fn state(b: &mut Battle<Recorder>, id: EffectId) -> crate::state::CellRef {
    let c = b
        .state
        .effects
        .alloc(Holder::mon(MonId(0)), Holder::mon(MonId(0)), id, 0);
    let r = b.state.effects.capture(c);
    b.scratch.current_state = Some(r);
    b.scratch.current_effect = EffectRef::Dex(id);
    r
}
pub(crate) fn numbers<T: std::str::FromStr>(s: &str) -> Vec<T>
where
    T::Err: std::fmt::Debug,
{
    s.split(',').map(|v| v.parse().unwrap()).collect()
}

pub(crate) fn check_scenario_teams(table: &str) {
    for row in table.lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<_> = row.split('\t').collect();
        assert_eq!(c.len(), 7);
        let seed = numbers::<u16>(c[1]).try_into().unwrap();
        let b = Battle::new(seed, c[2], c[3]).unwrap();
        assert_eq!(b.seed(), seed, "{}", c[0]);
        assert!(c[6].contains("|start"));
    }
}

pub(crate) fn replay_scenario(table: &str, name: &str) {
    let row = table
        .lines()
        .find(|r| r.split('\t').next() == Some(name))
        .unwrap();
    let c: Vec<_> = row.split('\t').collect();
    let mut b = Battle::from_players(
        numbers::<u16>(c[1]).try_into().unwrap(),
        ("Alice", c[2]),
        ("Bob", c[3]),
        TextLog::default(),
    )
    .unwrap();
    b.start().unwrap();
    for step in c[4].split(';').filter(|s| !s.is_empty()) {
        b.choose(if &step[..2] == "p1" { 0 } else { 1 }, &step[3..])
            .unwrap_or_else(|e| panic!("{name}: {step}: {e}"));
    }
    let mut log = Vec::new();
    b.drain_log(&mut log);
    for line in &mut log {
        if line.starts_with("|t:|") {
            *line = "|t:|".into();
        }
    }
    let expected: Vec<_> = c[6].split("\\n").collect();
    let mismatch = log
        .iter()
        .map(String::as_str)
        .zip(&expected)
        .position(|(a, e)| a != *e);
    assert_eq!(
        mismatch,
        None,
        "{name}: first mismatch: {:?}",
        mismatch.map(|i| (i, &log[i], expected[i]))
    );
    assert_eq!(log.len(), expected.len(), "{name}: log length");
    assert_eq!(
        b.seed(),
        numbers::<u16>(c[5]).as_slice(),
        "{name}: final RNG seed"
    );
}
