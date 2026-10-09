//! Ports data/items.ts:7713: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_WIDELENS;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_WIDELENS_ONSOURCEMODIFYACCURACY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_WIDELENS_ONSOURCEMODIFYACCURACY => {
            if matches!(
                b.event_arg(cx, 0),
                EventArg::Number(_) | EventArg::Relay(Relay::Number(_))
            ) {
                b.chain_modify(4505.0, 4096.0);
            }
        }
        _ => unreachable!("unexpected widelens callback"),
    }
    Relay::Undefined
}
