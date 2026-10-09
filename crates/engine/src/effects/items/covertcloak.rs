//! Ports data/items.ts:1201: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_COVERTCLOAK;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_COVERTCLOAK_ONMODIFYSECONDARIES];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_COVERTCLOAK_ONMODIFYSECONDARIES => {
            let EventArg::Relay(Relay::Secondaries(i)) = b.event_arg(cx, 0) else {
                panic!("Covert Cloak requires secondary array")
            };
            let original = *b.scratch_secondaries(i);
            let mut filtered = [None; 4];
            let mut len = 0;
            for s in original.into_iter().flatten() {
                if s.self_effect.is_some() {
                    filtered[len] = Some(s);
                    len += 1;
                }
            }
            Relay::Secondaries(b.stash_secondaries(filtered))
        }
        _ => unreachable!("unexpected covertcloak callback"),
    }
}
