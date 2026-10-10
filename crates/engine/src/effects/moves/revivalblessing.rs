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
pub const ID: EffectId = dex::MOVE_REVIVALBLESSING;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_REVIVALBLESSING_ONTRYHIT];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/moves.ts:15120-15124. PRNG: none; selection and revival are core-owned.
    let s = mon(b, cx, 0);
    if !b
        .state
        .pokemon
        .iter()
        .enumerate()
        .any(|(i, p)| MonId(i as u8).side() == s.side() && p.flags & mon_flags::FAINTED != 0)
    {
        return Relay::Bool(false);
    }
    Relay::Undefined
}
