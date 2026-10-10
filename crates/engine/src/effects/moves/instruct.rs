//! Faithful port of pinned Showdown 7332b60. Source/draw notes at each handler.
#![allow(unused_imports)]
use crate::effects::registry::moves_batonpass::support::{self, effect, handle, volatile};
use crate::{
    Battle,
    actions::*,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg as mon, optional_id},
    },
    event::{EffectRef, EventArg, HookCtx, Relay, RunEventOptions},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::{
        mon_flags,
        scratch::{OrderedBoosts, move_runtime},
    },
};
pub const ID: EffectId = dex::MOVE_INSTRUCT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_INSTRUCT_ONHIT];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/moves.ts:9650-9673. PRNG: resolveAction priority/target events; no direct draw.
    let t = mon(b, cx, 0);
    let s = mon(b, cx, 1);
    let p = b.state.pokemon[t.0 as usize];
    if p.last_move == EffectId::NONE || volatile(b, t, "dynamax") {
        return Relay::Bool(false);
    }
    let m = &dex::MOVES[(p.last_move.0 - dex::MOVE_START) as usize];
    if m.flags & (dex::FLAG_FAILINSTRUCT | dex::FLAG_CHARGE | dex::FLAG_RECHARGE) != 0
        || matches!(dex::effect(m.id).data.get(dex::FIELD_ISZ), Some(dex::DataValue::Text(s)) if !s.is_empty())
        || dex::effect(m.id).data.get(dex::FIELD_ISMAX) == Some(dex::DataValue::Bool(true))
        || volatile(b, t, "beakblast")
        || volatile(b, t, "focuspunch")
        || volatile(b, t, "shelltrap")
        || b.get_move_data(t, p.last_move).is_some_and(|m| m.pp == 0)
    {
        return Relay::Bool(false);
    }
    b.add(LogEntry::new(
        "-singleturn",
        &[LogArg::Mon(t), LogArg::Text("move: Instruct")],
        &[LogTag::Of(s)],
    ));
    let a = b
        .resolve_action(
            crate::sim::lifecycle::ActionChoice::for_move(
                t,
                p.last_move,
                Some(p.last_move_target_loc),
            ),
            false,
        )
        .entries[0];
    b.queue_prioritize_action(a, None, EffectRef::None);
    Relay::Undefined
}
