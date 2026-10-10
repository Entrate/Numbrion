mod support;
use crate::{dex, event::*, ids::*, state::scratch::OrderedBoosts};
#[test]
fn rattled_ignores_absent_attack_key_even_when_storage_is_nonzero() {
    let mut b = support::battle();
    for stale in [-1, 1] {
        let mut boosts = OrderedBoosts::default();
        boosts.len = 1;
        boosts.order[0] = 4;
        boosts.present = 1 << 4;
        boosts.values[0] = stale;
        boosts.values[4] = -1;
        let i = b.stash_boosts(boosts);
        let args = [
            EventArg::Relay(Relay::Boosts(i)),
            EventArg::Holder(Holder::mon(MonId(0))),
            EventArg::Holder(Holder::mon(MonId(6))),
            EventArg::Effect(EffectRef::Dex(dex::ABILITY_INTIMIDATE)),
        ];
        assert_eq!(
            support::invoke(&mut b, super::ID, "onAfterBoost", args),
            Relay::Undefined
        );
        b.release_relay(Relay::Boosts(i));
    }
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}
