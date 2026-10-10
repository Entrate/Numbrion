//! Private helpers shared only by recursive_switch and item_exchange ports.
use crate::{
    Battle,
    actions::MoveHandle,
    dex,
    event::{EffectRef, EventArg, HookCtx},
    ids::*,
    log::LogSink,
};
pub fn mon(m: MonId) -> EventArg {
    EventArg::Holder(Holder::mon(m))
}
/// `id` is `optional_id(dex::CONDITIONS_DATA, key)`, resolved by the caller at compile
/// time; NONE (a key outside the scope) never matches.
pub fn volatile<L: LogSink>(b: &Battle<L>, m: MonId, id: EffectId) -> bool {
    id != EffectId::NONE && b.get_volatile(m, id).is_some()
}
pub fn effect<L: LogSink>(b: &Battle<L>, cx: HookCtx, i: usize) -> EffectRef {
    match b.event_arg(cx, i) {
        EventArg::Move(h) => EffectRef::ActiveMove(h),
        EventArg::Effect(e) => e,
        _ => EffectRef::None,
    }
}
pub fn handle<L: LogSink>(b: &Battle<L>, cx: HookCtx, i: usize) -> MoveHandle {
    MoveHandle(crate::effects::support::move_arg(b, cx, i))
}
pub fn berry(id: EffectId) -> bool {
    id != EffectId::NONE
        && dex::effect(id).data.get(dex::FIELD_ISBERRY) == Some(dex::DataValue::Bool(true))
}
