mod support;
use super::ID;
use crate::{
    dex,
    event::*,
    ids::*,
    state::{present, scratch::move_runtime},
};
use support::*;
fn captured(b: &mut crate::Battle<crate::log::NoLog>) -> crate::state::CellId {
    let c = b
        .state
        .effects
        .alloc(Holder::mon(MonId(0)), Holder::mon(MonId(6)), ID, 0);
    b.scratch.current_state = Some(b.state.effects.pin(c));
    c
}
#[test]
fn berserk_payload_preserves_missing_false_true_and_multihit_truthiness() {
    let mut b = battle();
    let c = captured(&mut b);
    let item_args = [
        EventArg::Effect(EffectRef::Dex(dex::ITEM_SITRUSBERRY)),
        EventArg::Undefined,
        EventArg::Undefined,
        EventArg::Undefined,
    ];
    assert_eq!(
        invoke(&mut b, ID, "onTryEatItem", item_args),
        Relay::Undefined
    );
    assert_eq!(b.state.effects.cells[c.0 as usize].present & (1 << 8), 0);
    for (flags, hits, expected) in [
        (0, [0, 0], false),
        (move_runtime::MULTIHIT_PRESENT, [0, 0], false),
        (move_runtime::MULTIHIT_PRESENT, [2, 2], true),
        (
            move_runtime::MULTIHIT_PRESENT | move_runtime::MULTIHIT_RANGE,
            [0, 0],
            true,
        ),
    ] {
        let m = b.scratch.moves[0].as_mut().unwrap();
        m.runtime_flags = flags;
        m.multihit = hits;
        assert_eq!(
            invoke(&mut b, ID, "onDamage", numeric_args(20.0)),
            Relay::Undefined
        );
        assert_ne!(b.state.effects.cells[c.0 as usize].present & (1 << 8), 0);
        assert_eq!(
            b.state.effects.cells[c.0 as usize].payload.words[0],
            u32::from(expected)
        );
        assert_eq!(
            invoke(&mut b, ID, "onTryEatItem", item_args),
            Relay::Bool(expected)
        );
    }
    let nonhealing = [
        EventArg::Effect(EffectRef::Dex(dex::ITEM_LEFTOVERS)),
        EventArg::Undefined,
        EventArg::Undefined,
        EventArg::Undefined,
    ];
    assert_eq!(
        invoke(&mut b, ID, "onTryEatItem", nonhealing),
        Relay::Bool(true)
    );
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}
#[test]
fn after_secondary_writes_flag_before_missing_source_and_zero_damage_guards() {
    let mut b = battle();
    let c = captured(&mut b);
    let args = [
        EventArg::Holder(Holder::mon(MonId(0))),
        EventArg::Undefined,
        EventArg::Move(0),
        EventArg::Undefined,
    ];
    assert_eq!(
        invoke(&mut b, ID, "onAfterMoveSecondary", args),
        Relay::Undefined
    );
    assert_ne!(b.state.effects.cells[c.0 as usize].present & (1 << 8), 0);
    assert_eq!(b.state.effects.cells[c.0 as usize].payload.words[0], 1);
    assert_eq!(
        b.state.effects.cells[c.0 as usize].target,
        Holder::mon(MonId(6))
    );
    // Retired captured listener payload stays accessible through the pin.
    b.state.effects.release(c);
    b.state.effects.cells[c.0 as usize].payload.words[0] = 0;
    assert_eq!(
        invoke(&mut b, ID, "onAfterMoveSecondary", args),
        Relay::Undefined
    );
    assert_eq!(b.state.effects.cells[c.0 as usize].payload.words[0], 1);
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}
