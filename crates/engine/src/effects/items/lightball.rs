//! Ports data/items.ts:3421: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_LIGHTBALL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_LIGHTBALL_ONMODIFYATK,
    dex::HOOK_ITEM_LIGHTBALL_ONMODIFYSPA,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_LIGHTBALL_ONMODIFYATK => {
            // Dynamax is unavailable in the pinned Gen 9 format.
            if dex::species(b.state.pokemon[mon_arg(b, cx, 1).0 as usize].base_species)
                .base_species_name
                == "Pikachu"
            {
                b.chain_modify(2.0, 1.0);
            }
        }
        dex::HOOK_ITEM_LIGHTBALL_ONMODIFYSPA => {
            // Dynamax is unavailable in the pinned Gen 9 format.
            if dex::species(b.state.pokemon[mon_arg(b, cx, 1).0 as usize].base_species)
                .base_species_name
                == "Pikachu"
            {
                b.chain_modify(2.0, 1.0);
            }
        }
        _ => unreachable!("unexpected lightball callback"),
    }
    Relay::Undefined
}
