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
pub const ID: EffectId = dex::MOVE_POLTERGEIST;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_POLTERGEIST_ONTRY,
    dex::HOOK_MOVE_POLTERGEIST_ONTRYHIT,
];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:13604-13606. PRNG: none.
        dex::HOOK_MOVE_POLTERGEIST_ONTRY => {
            let t = mon(b, cx, 1);
            Relay::Bool(b.state.pokemon[t.0 as usize].item != EffectId::NONE)
        }
        // data/moves.ts:13607-13609. PRNG: none.
        dex::HOOK_MOVE_POLTERGEIST_ONTRYHIT => {
            let t = mon(b, cx, 0);
            let item = b.state.pokemon[t.0 as usize].item;
            b.add(LogEntry::new(
                "-activate",
                &[
                    LogArg::Mon(t),
                    LogArg::Text("move: Poltergeist"),
                    LogArg::Effect(EffectRef::Dex(item)),
                ],
                &[],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected Poltergeist hook"),
    }
}
