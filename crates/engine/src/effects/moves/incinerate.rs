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
pub const ID: EffectId = dex::MOVE_INCINERATE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_INCINERATE_ONHIT];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/moves.ts:9533-9538. PRNG: takeItem events only.
    let t = mon(b, cx, 0);
    let s = mon(b, cx, 1);
    let item = b.state.pokemon[t.0 as usize].item;
    if item != EffectId::NONE
        && (support::berry(item) || dex::ITEMS[(item.0 - dex::ITEM_START) as usize].gem)
        && b.take_item(t, Some(s)).truthy()
    {
        b.add(LogEntry::new(
            "-enditem",
            &[LogArg::Mon(t), LogArg::Effect(EffectRef::Dex(item))],
            &[LogTag::From(EffectRef::Dex(ID))],
        ));
    }
    Relay::Undefined
}
