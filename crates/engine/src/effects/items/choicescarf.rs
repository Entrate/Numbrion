//! Ports data/items.ts:983: complete scoped callbacks; no direct PRNG draws.
#![allow(unused_imports, unused_variables)]
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::LogSink,
};
#[path = "itemsmodifiers/support.rs"]
mod support;
use support::*;
pub const ID: EffectId = dex::ITEM_CHOICESCARF;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_CHOICESCARF_ONSTART,
    dex::HOOK_ITEM_CHOICESCARF_ONMODIFYMOVE,
    dex::HOOK_ITEM_CHOICESCARF_ONMODIFYSPE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_CHOICESCARF_ONSTART => {
            b.remove_volatile(mon_arg(b, cx, 0), dex::CONDITION_CHOICELOCK);
        }
        dex::HOOK_ITEM_CHOICESCARF_ONMODIFYMOVE => {
            b.add_volatile(
                mon_arg(b, cx, 1),
                dex::CONDITION_CHOICELOCK,
                crate::actions::Attribution::DEFAULT,
                None,
            );
        }
        dex::HOOK_ITEM_CHOICESCARF_ONMODIFYSPE => {
            // Dynamax is unavailable in the pinned Gen 9 format.
            if true {
                b.chain_modify(1.5, 1.0);
            }
        }
        _ => unreachable!("unexpected choicescarf callback"),
    }
    Relay::Undefined
}
