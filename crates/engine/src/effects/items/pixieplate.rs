//! Ports data/items.ts:4616: complete scoped callbacks; no direct PRNG draws.
#![allow(unused_imports, unused_variables, unreachable_code)]
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
pub const ID: EffectId = dex::ITEM_PIXIEPLATE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_PIXIEPLATE_ONBASEPOWER,
    dex::HOOK_ITEM_PIXIEPLATE_ONTAKEITEM,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_PIXIEPLATE_ONBASEPOWER => {
            if let Some(i) = optional_move(b, cx, 3) {
                let mv = *move_overlay(b, i);
                if mv.move_type == dex::type_id("Fairy").unwrap() {
                    b.chain_modify(4915.0, 4096.0);
                }
            }
        }
        dex::HOOK_ITEM_PIXIEPLATE_ONTAKEITEM => {
            let source = Battle::<L>::arg_mon(b.event_arg(cx, 2));
            return Relay::Bool(
                !source.is_some_and(|m| base_num(b, m) == 493.0)
                    && base_num(b, mon_arg(b, cx, 1)) != 493.0,
            );
        }
        _ => unreachable!("unexpected pixieplate callback"),
    }
    Relay::Undefined
}
