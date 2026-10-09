//! Compile the public adapter/training surface without invoking unfinished battle behavior.
use engine::{
    Battle,
    event::{EffectRef, SyntheticEffect},
    ids::{EffectId, EffectToken, SideId},
    log::{LogSink, TextLog},
    sim::{BattleError, ChoiceError, LegalActions, Outcome},
};
#[test]
fn adapter_surface_compiles() {
    let _: fn(&mut Battle<TextLog>) -> Result<(), BattleError> = Battle::start;
    let _: fn(&mut Battle<TextLog>, usize, &str) -> Result<(), ChoiceError> = Battle::choose;
    let _: fn(&Battle<TextLog>, usize) -> Option<String> = Battle::request_json;
    let _: fn(&Battle<TextLog>) -> Option<Outcome> = Battle::outcome;
    let _: fn(&mut Battle<TextLog>, &mut Vec<String>) = Battle::drain_log;
    let _: fn(&Battle<TextLog>) -> [u16; 4] = Battle::seed;
    let _: fn(&Battle<TextLog>) -> u32 = Battle::turn;
    fn training<L: LogSink>() {
        let _: fn(&Battle<L>, SideId) -> LegalActions = Battle::legal_actions;
    }
    training::<engine::log::NoLog>();
}
#[test]
fn faint_attribution_keeps_views_and_synthetic_effects() {
    let id = EffectId(12);
    for effect in [
        EffectRef::None,
        EffectRef::Dex(id),
        EffectRef::SpeciesCondition(id),
        EffectRef::MoveCondition(id),
        EffectRef::AbilityCondition(id),
        EffectRef::ItemCondition(id),
    ] {
        assert_eq!(EffectToken::from_ref(effect).resolve(), effect);
    }
    for s in [
        SyntheticEffect::Format,
        SyntheticEffect::Confused,
        SyntheticEffect::StruggleRecoil,
        SyntheticEffect::Recharge,
        SyntheticEffect::Fainted,
        SyntheticEffect::MindBlownRecoil,
    ] {
        assert_eq!(
            EffectToken::from_ref(EffectRef::Synthetic(s)).resolve(),
            EffectRef::Synthetic(s)
        );
    }
    assert!(std::panic::catch_unwind(|| EffectToken::from_ref(EffectRef::ActiveMove(0))).is_err());
}
