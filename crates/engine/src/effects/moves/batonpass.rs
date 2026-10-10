//! Faithful port of pinned Showdown 7332b60. Source/draw notes at each handler.
#![allow(unused_imports)]
use crate::effects::registry::moves_batonpass::support::{effect, handle, volatile};
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
#[path = "batonpass/support.rs"]
pub mod support;
pub const ID: EffectId = dex::MOVE_BATONPASS;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_BATONPASS_ONHIT,
    dex::HOOK_MOVE_BATONPASS_SELF_ONHIT,
];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:1101-1107. PRNG: none.
        dex::HOOK_MOVE_BATONPASS_ONHIT => {
            let t = mon(b, cx, 0);
            if b.can_switch(t.side()) == 0 || volatile(b, t, "commanded") {
                b.attr_last_move(MoveLineEdit::Still);
                b.add(LogEntry::new("-fail", &[LogArg::Mon(t)], &[]));
                return Relay::NotFail;
            }
            Relay::Undefined
        }
        // data/moves.ts:1109-1111. PRNG: none.
        dex::HOOK_MOVE_BATONPASS_SELF_ONHIT => {
            let t = mon(b, cx, 0);
            b.state.pokemon[t.0 as usize].flags |= mon_flags::SKIP_BEFORE_SWITCH_OUT;
            Relay::Undefined
        }
        _ => panic!("unexpected Baton Pass hook"),
    }
}

#[cfg(test)]
#[path = "batonpass/tests.rs"]
mod tests;
