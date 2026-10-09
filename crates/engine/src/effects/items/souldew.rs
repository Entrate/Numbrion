//! Ports data/items.ts:5883: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_SOULDEW;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_SOULDEW_ONBASEPOWER];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_SOULDEW_ONBASEPOWER => {
            if let Some(i) = optional_move(b, cx, 3) {
                let mv = *move_overlay(b, i);
                if (base_num(b, mon_arg(b, cx, 1)) == 380.0
                    || base_num(b, mon_arg(b, cx, 1)) == 381.0)
                    && (mv.move_type == dex::type_id("Psychic").unwrap()
                        || mv.move_type == dex::type_id("Dragon").unwrap())
                {
                    b.chain_modify(4915.0, 4096.0);
                }
            }
        }
        _ => unreachable!("unexpected souldew callback"),
    }
    Relay::Undefined
}
