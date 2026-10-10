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
pub const ID: EffectId = dex::ABILITY_FRISK;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_FRISK_ONSTART];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/abilities.ts:1547-1553. PRNG: none. Retains foes() position order.
    let s = mon(b, cx, 0);
    let foes = b.foes(s, false);
    for &target in &foes.entries[..foes.len as usize] {
        let HitTarget::Pokemon(t) = target else {
            continue;
        };
        let item = b.state.pokemon[t.0 as usize].item;
        if item != EffectId::NONE {
            b.add(LogEntry::new(
                "-item",
                &[LogArg::Mon(t), LogArg::Effect(EffectRef::Dex(item))],
                &[LogTag::From(EffectRef::Dex(ID)), LogTag::Of(s)],
            ));
        }
    }
    Relay::Undefined
}
