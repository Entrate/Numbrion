#![allow(dead_code)]
use crate::{
    Battle, dex,
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::MonId,
    log::LogSink,
};
pub fn base_num<L: LogSink>(b: &Battle<L>, m: MonId) -> f64 {
    match dex::effect(b.state.pokemon[m.0 as usize].base_species)
        .data
        .get(dex::FIELD_NUM)
    {
        Some(dex::DataValue::Number(n)) => n,
        _ => panic!("species number absent"),
    }
}
pub fn optional_move<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> Option<u8> {
    match b.event_arg(cx, index) {
        EventArg::Move(i)
        | EventArg::Effect(EffectRef::ActiveMove(i))
        | EventArg::Relay(Relay::ActiveMove(i)) => Some(i),
        EventArg::Undefined | EventArg::Null => None,
        _ => panic!("expected optional live move"),
    }
}
