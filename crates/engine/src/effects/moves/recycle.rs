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
pub const ID: EffectId = dex::MOVE_RECYCLE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_RECYCLE_ONHIT];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/moves.ts:14830-14837. PRNG: setItem Start events only.
    let t = mon(b, cx, 0);
    let s = mon(b, cx, 1);
    let e = effect(b, cx, 2);
    let p = b.state.pokemon[t.0 as usize];
    if p.item != EffectId::NONE || p.last_item == EffectId::NONE {
        return Relay::Bool(false);
    }
    b.state.pokemon[t.0 as usize].last_item = EffectId::NONE;
    b.add(LogEntry::new(
        "-item",
        &[LogArg::Mon(t), LogArg::Effect(EffectRef::Dex(p.last_item))],
        &[LogTag::From(EffectRef::Dex(ID))],
    ));
    b.set_item(t, p.last_item, Attribution::from_move(s, e));
    Relay::Undefined
}
