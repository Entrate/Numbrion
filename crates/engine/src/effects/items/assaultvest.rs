//! Ports data/items.ts:313: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_ASSAULTVEST;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_ASSAULTVEST_ONMODIFYSPD,
    dex::HOOK_ITEM_ASSAULTVEST_ONDISABLEMOVE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_ASSAULTVEST_ONMODIFYSPD => {
            // Dynamax is unavailable in the pinned Gen 9 format.
            if true {
                b.chain_modify(1.5, 1.0);
            }
        }
        dex::HOOK_ITEM_ASSAULTVEST_ONDISABLEMOVE => {
            let m = mon_arg(b, cx, 0);
            let slots = b.state.pokemon[m.0 as usize].move_slots();
            let mut ids = [EffectId::NONE; 4];
            let len = slots.len();
            for (i, slot) in slots.iter().enumerate() {
                ids[i] = slot.id;
            }
            for id in ids.into_iter().take(len) {
                if dex::move_data(id).category == dex::Category::Status {
                    // Me First is absent from this Gen 9 scope.
                    b.disable_move(m, id, false, b.scratch.current_effect);
                }
            }
        }
        _ => unreachable!("unexpected assaultvest callback"),
    }
    Relay::Undefined
}
