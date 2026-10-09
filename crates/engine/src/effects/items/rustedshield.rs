//! Ports data/items.ts:5403: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_RUSTEDSHIELD;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_RUSTEDSHIELD_ONTAKEITEM];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_RUSTEDSHIELD_ONTAKEITEM => {
            let source = Battle::<L>::arg_mon(b.event_arg(cx, 2));
            Relay::Bool(
                !source.is_some_and(|m| base_num(b, m) == 889.0)
                    && base_num(b, mon_arg(b, cx, 1)) != 889.0,
            )
        }
        _ => unreachable!("unexpected rustedshield callback"),
    }
}
