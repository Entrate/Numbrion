//! Ports data/items.ts:2808: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_HEARTHFLAMEMASK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_HEARTHFLAMEMASK_ONBASEPOWER,
    dex::HOOK_ITEM_HEARTHFLAMEMASK_ONTAKEITEM,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_HEARTHFLAMEMASK_ONBASEPOWER => {
            let mv = *move_overlay(b, move_arg(b, cx, 3));
            if dex::species(b.state.pokemon[mon_arg(b, cx, 1).0 as usize].base_species)
                .name
                .starts_with("Ogerpon-Hearthflame")
            {
                b.chain_modify(4915.0, 4096.0);
            }
        }
        dex::HOOK_ITEM_HEARTHFLAMEMASK_ONTAKEITEM => {
            return Relay::Bool(
                dex::species(b.state.pokemon[mon_arg(b, cx, 1).0 as usize].base_species)
                    .base_species_name
                    != "Ogerpon",
            );
        }
        _ => unreachable!("unexpected hearthflamemask callback"),
    }
    Relay::Undefined
}
