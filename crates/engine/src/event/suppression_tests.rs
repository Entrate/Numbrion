#[allow(dead_code, unused_imports)]
#[path = "../effects/abilities/passiveoffense/support.rs"]
mod setup;
use crate::{
    actions::MoveHandle,
    dex,
    event::*,
    ids::*,
    state::{Status, mon_flags, present, scratch::move_runtime},
};
fn event(key: &str) -> EventId {
    match key {
        "Start" => EventId::Start,
        "End" => EventId::End,
        "SwitchIn" => EventId::SwitchIn,
        "TakeItem" => EventId::TakeItem,
        "SetAbility" => EventId::SetAbility,
        "Update" => EventId::Update,
        "BasePower" => EventId::BasePower,
        "Weather" => EventId::Weather,
        "FieldStart" => EventId::FieldStart,
        "FieldResidual" => EventId::FieldResidual,
        "FieldEnd" => EventId::FieldEnd,
        "Residual" => EventId::Residual,
        "AfterSetStatus" => EventId::AfterSetStatus,
        _ => panic!("unknown probe event"),
    }
}
fn listener(effect: EffectRef, event: EventId) -> Listener {
    Listener {
        effect,
        selector: HookSelector {
            event,
            rel: dex::HookRel::On,
            mode: CollectMode::Callback,
        },
        hook: None,
        state: None,
        holder: Holder::mon(MonId(0)),
        priority: Priority::default(),
        target_index: 0,
        end: EndHandler::None,
    }
}
#[test]
fn single_and_run_suppression_match_pinned_dispatcher() {
    let mut b = setup::battle();
    b.state.pokemon[1] = b.state.pokemon[0];
    let cloud = b.state.effects.alloc(
        Holder::mon(MonId(1)),
        Holder::mon(MonId(1)),
        dex::ABILITY_CLOUDNINE,
        0,
    );
    b.state.pokemon[1].ability_state = cloud;
    b.state.pokemon[1].ability = dex::ABILITY_CLOUDNINE;
    let original = b.state;
    let mut count = 0;
    for line in include_str!("suppression_vectors.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let c: Vec<_> = line.split('\t').collect();
        let kind = match c[0] {
            "Ability" => EffectKind::Ability,
            "Item" => EffectKind::Item,
            "Status" | "Weather" => EffectKind::Condition,
            "Rule" => EffectKind::Rule,
            _ => panic!("unknown effect kind"),
        };
        let id = dex::lookup(kind, c[1]).unwrap();
        let effect = EffectRef::Dex(id);
        let e = event(c[2]);
        let num = |i: usize| c[i].parse::<u8>().unwrap();
        b.state = original;
        b.state.pokemon[0].flags = if num(3) == 1 { mon_flags::ACTIVE } else { 0 };
        b.state.pokemon[0].status = if num(6) == 1 {
            Status::Burn
        } else {
            Status::None
        };
        b.state.pokemon[6].flags = if num(4) == 1 { mon_flags::ACTIVE } else { 0 };
        b.scratch.active_pokemon = match num(4) {
            0 => MonId::NONE,
            2 => MonId(0),
            _ => MonId(6),
        };
        b.scratch.active_move = if num(5) == 1 {
            MoveHandle(0)
        } else {
            MoveHandle::NONE
        };
        b.scratch.moves[0].as_mut().unwrap().runtime_flags = if num(5) == 1 {
            move_runtime::IGNORE_ABILITY
        } else {
            0
        };
        let weather = num(7);
        b.state.sides[0].active = if weather > 0 {
            [MonId(0), MonId(1)]
        } else {
            [MonId(0), MonId::NONE]
        };
        b.state.pokemon[1].flags = match weather {
            3 => mon_flags::ACTIVE | mon_flags::FAINTED,
            4 => 0,
            _ => mon_flags::ACTIVE,
        };
        b.state.effects.cells[cloud.0 as usize].present =
            if weather == 2 { present::ENDING } else { 0 };
        let seed = b.seed();
        let lines = b.scratch.unsent_lines;
        assert_eq!(
            b.single_event_suppressed(e, effect, EventArg::Holder(Holder::mon(MonId(0)))),
            num(8) == 1,
            "single {line}"
        );
        assert_eq!(
            b.run_event_suppressed(e, listener(effect, e)),
            num(9) == 1,
            "run {line}"
        );
        assert_eq!(b.seed(), seed);
        assert_eq!(b.scratch.unsent_lines, lines);
        count += 1;
    }
    assert_eq!(count, 12480);
}
#[test]
fn live_move_alias_and_context_clearing_preserve_success_and_failure() {
    let mut b = setup::battle();
    b.set_active_move(Some(MoveHandle(0)), Some(MonId(0)), None);
    assert_eq!(b.scratch.active_target, MonId(0));
    b.active_move_mut(MoveHandle(0)).base_power = 77.0;
    assert_eq!(b.active_move(MoveHandle(0)).base_power, 77.0);
    b.clear_active_move(true);
    assert_eq!(b.state.last_move, EffectId::NONE);
    assert_eq!(b.scratch.active_pokemon, MonId::NONE);
    b.set_active_move(Some(MoveHandle(0)), Some(MonId(0)), Some(MonId(6)));
    b.clear_active_move(false);
    assert_eq!(b.state.last_move, dex::MOVE_THUNDERBOLT);
    assert_eq!(b.scratch.active_move, MoveHandle::NONE);
    assert!(b.scratch.moves[0].is_some());
    b.set_active_move(None, Some(MonId(0)), Some(MonId(6)));
    b.clear_active_move(false);
    assert_eq!(b.scratch.active_pokemon, MonId(0));
    assert_eq!(b.scratch.active_target, MonId(6));
}
#[test]
fn volatile_lookup_is_read_only_and_uses_current_list_identity() {
    let mut b = setup::battle();
    let m = MonId(0);
    let id = dex::CONDITION_CONFUSION;
    assert!(b.get_volatile(m, id).is_none());
    let cell = b.state.effects.alloc(Holder::mon(m), Holder::mon(m), id, 0);
    b.state.pokemon[0].volatiles.push(cell);
    let seed = b.seed();
    assert_eq!(b.get_volatile(m, id), Some(cell));
    b.state.pokemon[0].volatiles.remove(0);
    assert!(b.get_volatile(m, id).is_none());
    assert_eq!(b.seed(), seed);
}
