use crate::{
    Battle,
    dex::{self, Category},
    event::*,
    ids::*,
    log::NoLog,
    state::scratch::*,
};
pub fn battle() -> Battle<NoLog> {
    let packed = "Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric";
    let mut b = Battle::new([1, 2, 3, 4], packed, packed).unwrap();
    b.state.pokemon[0].hp = 40;
    b.state.pokemon[0].max_hp = 120;
    b.state.pokemon[0].status = crate::state::Status::Poison;
    b.scratch.moves[0] = Some(active());
    b.scratch.frames[0] = Some(EventFrame {
        event: EventId::BasePower,
        target: EventArg::Holder(Holder::mon(MonId(0))),
        source: EventArg::Holder(Holder::mon(MonId(6))),
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
    b
}
pub fn effects(base: &'static dex::MoveEffects) -> MoveEffectsScratch {
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
pub fn active() -> ActiveMove {
    let m = dex::move_data(dex::MOVE_THUNDERBOLT);
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
        category: Category::Physical,
        target: m.target,
        priority: m.priority,
        crit_ratio: m.crit_ratio,
        hit: 0,
        multihit: m.multihit,
    }
}
pub fn invoke<L: crate::log::LogSink>(
    b: &mut Battle<L>,
    id: EffectId,
    key: &str,
    args: [EventArg; 4],
) -> Relay {
    let h = &dex::MANIFESTS[id.0 as usize];
    let i = h
        .hooks()
        .iter()
        .position(|h| h.key == key && h.site.is_empty())
        .unwrap();
    let hook = dex::HookId(h.hooks_start + i as u16);
    b.call_hook(
        hook,
        CallArgs {
            values: args,
            len: 4,
        },
    )
}
pub fn numeric_args(n: f64) -> [EventArg; 4] {
    [
        EventArg::Number(n),
        EventArg::Holder(Holder::mon(MonId(0))),
        EventArg::Holder(Holder::mon(MonId(6))),
        EventArg::Move(0),
    ]
}
pub fn move_args() -> [EventArg; 4] {
    [
        EventArg::Move(0),
        EventArg::Holder(Holder::mon(MonId(0))),
        EventArg::Holder(Holder::mon(MonId(6))),
        EventArg::Undefined,
    ]
}
