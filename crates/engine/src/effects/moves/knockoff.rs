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
pub const ID: EffectId = dex::MOVE_KNOCKOFF;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_KNOCKOFF_ONBASEPOWER,
    dex::HOOK_MOVE_KNOCKOFF_ONAFTERHIT,
];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:9968-9974. PRNG: nested item TakeItem event only; deliberately not ability TakeItem.
        dex::HOOK_MOVE_KNOCKOFF_ONBASEPOWER => {
            let t = mon(b, cx, 2);
            let h = handle(b, cx, 3);
            let p = b.state.pokemon[t.0 as usize];
            if !b
                .single_event(
                    EventId::TakeItem,
                    EffectRef::Dex(p.item),
                    Some(b.state.effects.capture(p.item_state)),
                    support::mon(t),
                    support::mon(t),
                    EffectRef::ActiveMove(h.0),
                    Relay::Effect(p.item),
                    None,
                )
                .truthy()
            {
                return Relay::Undefined;
            }
            if p.item != EffectId::NONE {
                b.chain_modify(3., 2.);
            }
            Relay::Undefined
        }
        // data/moves.ts:9975-9980. PRNG: takeItem events only; omitted source defaults to target.
        dex::HOOK_MOVE_KNOCKOFF_ONAFTERHIT => {
            let t = mon(b, cx, 0);
            let s = mon(b, cx, 1);
            if let Relay::Effect(item) = b.take_item(t, None) {
                b.add(LogEntry::new(
                    "-enditem",
                    &[LogArg::Mon(t), LogArg::Effect(EffectRef::Dex(item))],
                    &[LogTag::From(EffectRef::Dex(ID)), LogTag::Of(s)],
                ));
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Knock Off hook"),
    }
}
