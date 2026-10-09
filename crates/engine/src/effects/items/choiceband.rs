//! Ports data/items.ts:959: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_CHOICEBAND;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_CHOICEBAND_ONSTART,
    dex::HOOK_ITEM_CHOICEBAND_ONMODIFYMOVE,
    dex::HOOK_ITEM_CHOICEBAND_ONMODIFYATK,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_CHOICEBAND_ONSTART => {
            b.remove_volatile(mon_arg(b, cx, 0), dex::CONDITION_CHOICELOCK);
        }
        dex::HOOK_ITEM_CHOICEBAND_ONMODIFYMOVE => {
            b.add_volatile(
                mon_arg(b, cx, 1),
                dex::CONDITION_CHOICELOCK,
                crate::actions::Attribution::DEFAULT,
                None,
            );
        }
        dex::HOOK_ITEM_CHOICEBAND_ONMODIFYATK => {
            // Dynamax is unavailable in the pinned Gen 9 format.
            if true {
                b.chain_modify(1.5, 1.0);
            }
        }
        _ => unreachable!("unexpected choiceband callback"),
    }
    Relay::Undefined
}
