#[allow(dead_code)]
#[path = "../passiveoffense/support.rs"]
mod support;
use super::host::{BattleHost, Hit, Host};
use crate::{dex, event::*, ids::*, state::present};
fn context(b: &mut crate::Battle<crate::log::NoLog>, id: EffectId, args: [EventArg; 4]) -> HookCtx {
    let cell = b
        .state
        .effects
        .alloc(Holder::mon(MonId(0)), Holder::mon(MonId(0)), id, 0);
    b.scratch.calls[0] = Some(CallArgs {
        values: args,
        len: 4,
    });
    HookCtx {
        frame: 0,
        call_args: 0,
        active_move: 0,
        state: b.state.effects.capture(cell),
        holder: Holder::mon(MonId(0)),
        target: args[1],
        source: args[2],
        source_effect: EffectRef::ActiveMove(0),
        relay: Relay::Undefined,
        target_index: 0,
    }
}
#[test]
fn real_adapter_reads_ally_side_and_layer_presence() {
    let mut b = support::battle();
    let mut args = support::numeric_args(10.0);
    args[1] = EventArg::Holder(Holder::mon(MonId(0)));
    args[2] = EventArg::Holder(Holder::mon(MonId(1)));
    let cx = context(&mut b, dex::ABILITY_TOXICDEBRIS, args);
    let x = Hit::from_battle(&b, cx, dex::ABILITY_TOXICDEBRIS);
    assert_eq!(x.side, SideId(1));
    assert!(x.layers.is_none());
    assert!(x.physical);
    let cell = b.state.effects.alloc(
        Holder::side(SideId(1)),
        Holder::side(SideId(1)),
        dex::CONDITION_TOXICSPIKES,
        1,
    );
    b.state.sides[1].conditions.push(cell);
    assert_eq!(
        Hit::from_battle(&b, cx, dex::ABILITY_TOXICDEBRIS).layers,
        Some(u32::MAX)
    );
    b.state.effects.cells[cell.0 as usize].present |= 1 << present::CUSTOM_START;
    b.state.effects.cells[cell.0 as usize].payload.words[0] = 2;
    assert_eq!(
        Hit::from_battle(&b, cx, dex::ABILITY_TOXICDEBRIS).layers,
        Some(2)
    );
    b.scratch.calls[0].as_mut().unwrap().values[2] = EventArg::Holder(Holder::mon(MonId(6)));
    assert_eq!(
        Hit::from_battle(&b, cx, dex::ABILITY_TOXICDEBRIS).side,
        SideId(1)
    );
}
#[test]
fn real_adapter_preserves_captured_owner_and_disable_guard() {
    let mut b = support::battle();
    let mut args = support::numeric_args(10.0);
    args[1] = EventArg::Holder(Holder::mon(MonId(0)));
    args[2] = EventArg::Holder(Holder::mon(MonId(6)));
    let cx = context(&mut b, dex::ABILITY_CURSEDBODY, args);
    let disable = b.state.effects.alloc(
        Holder::mon(MonId(6)),
        Holder::mon(MonId(6)),
        dex::CONDITION_DISABLE,
        1,
    );
    b.state.pokemon[6].volatiles.push(disable);
    let x = Hit::from_battle(&b, cx, dex::ABILITY_CURSEDBODY);
    assert_eq!(x.owner, MonId(0));
    assert!(x.disabled);
    let seed = b.seed();
    let mut host = BattleHost {
        b: &mut b,
        move_index: Some(0),
    };
    assert_eq!(
        host.random(100),
        crate::prng::Prng::from_seed(seed).random(100)
    );
    assert_ne!(b.seed(), seed);
}
#[test]
fn synchronize_source_object_survives_tokens_without_ability_hooks() {
    let b = support::battle();
    for status in [
        dex::CONDITION_BRN,
        dex::CONDITION_PAR,
        dex::CONDITION_PSN,
        dex::CONDITION_TOX,
    ] {
        let effect = EffectRef::Synchronize(status);
        assert_eq!(EffectToken::from_ref(effect).resolve(), effect);
        assert_eq!(b.event_effect_id(effect), dex::ABILITY_SYNCHRONIZE);
        assert_eq!(b.event_effect_type(effect), dex::EffectType::Undefined);
        assert_eq!(b.synthetic_status(effect), Some(status));
        assert!(
            b.event_hook(effect, EventId::AfterSetStatus, dex::HookRel::On)
                .is_none()
        );
    }
}
