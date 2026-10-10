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
pub const ID: EffectId = dex::ABILITY_STICKYHOLD;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_STICKYHOLD_ONTAKEITEM];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/abilities.ts:4624-4631. PRNG: none.
    assert_ne!(
        b.scratch.active_move,
        MoveHandle::NONE,
        "Battle.activeMove is null"
    );
    let t = mon(b, cx, 1);
    let s = Battle::<L>::arg_mon(b.event_arg(cx, 2));
    let p = b.state.pokemon[t.0 as usize];
    const STICKYBARB: EffectId = optional_id(dex::ITEMS_DATA, "stickybarb");
    if p.hp == 0 || (STICKYBARB != EffectId::NONE && p.item == STICKYBARB) {
        return Relay::Undefined;
    }
    if s.is_some_and(|s| s != t) || b.active_move(b.scratch.active_move).id == dex::MOVE_KNOCKOFF {
        b.add(LogEntry::new(
            "-activate",
            &[LogArg::Mon(t), LogArg::Text("ability: Sticky Hold")],
            &[],
        ));
        return Relay::Bool(false);
    }
    Relay::Undefined
}
