//! Ports data/items.ts:1868: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_EVIOLITE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_EVIOLITE_ONMODIFYDEF,
    dex::HOOK_ITEM_EVIOLITE_ONMODIFYSPD,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_EVIOLITE_ONMODIFYDEF => {
            // Dynamax is unavailable in the pinned Gen 9 format.
            if dex::effect(b.state.pokemon[mon_arg(b, cx, 1).0 as usize].base_species)
                .data
                .get(dex::FIELD_NFE)
                == Some(dex::DataValue::Bool(true))
            {
                b.chain_modify(1.5, 1.0);
            }
        }
        dex::HOOK_ITEM_EVIOLITE_ONMODIFYSPD => {
            // Dynamax is unavailable in the pinned Gen 9 format.
            if dex::effect(b.state.pokemon[mon_arg(b, cx, 1).0 as usize].base_species)
                .data
                .get(dex::FIELD_NFE)
                == Some(dex::DataValue::Bool(true))
            {
                b.chain_modify(1.5, 1.0);
            }
        }
        _ => unreachable!("unexpected eviolite callback"),
    }
    Relay::Undefined
}
