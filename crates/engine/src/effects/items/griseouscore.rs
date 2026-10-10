//! Ports data/items.ts:2660: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_GRISEOUSCORE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_GRISEOUSCORE_ONBASEPOWER,
    dex::HOOK_ITEM_GRISEOUSCORE_ONTAKEITEM,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_GRISEOUSCORE_ONBASEPOWER => {
            let mv = *move_overlay(b, move_arg(b, cx, 3));
            if (base_num(b, mon_arg(b, cx, 1)) == 487.0)
                && (mv.move_type == dex::TYPE_GHOST
                    || mv.move_type == dex::TYPE_DRAGON)
            {
                b.chain_modify(4915.0, 4096.0);
            }
        }
        dex::HOOK_ITEM_GRISEOUSCORE_ONTAKEITEM => {
            let source = Battle::<L>::arg_mon(b.event_arg(cx, 2));
            return Relay::Bool(
                !source.is_some_and(|m| base_num(b, m) == 487.0)
                    && base_num(b, mon_arg(b, cx, 1)) != 487.0,
            );
        }
        _ => unreachable!("unexpected griseouscore callback"),
    }
    Relay::Undefined
}
