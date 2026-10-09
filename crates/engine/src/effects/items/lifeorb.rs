//! Ports data/items.ts:3404: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_LIFEORB;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_LIFEORB_ONMODIFYDAMAGE,
    dex::HOOK_ITEM_LIFEORB_ONAFTERMOVESECONDARYSELF,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_LIFEORB_ONMODIFYDAMAGE => {
            b.chain_modify(5324.0, 4096.0);
        }
        dex::HOOK_ITEM_LIFEORB_ONAFTERMOVESECONDARYSELF => {
            if let Some(m) = Battle::<L>::arg_mon(b.event_arg(cx, 0)) {
                if Some(m) != Battle::<L>::arg_mon(b.event_arg(cx, 1)) {
                    if let Some(i) = optional_move(b, cx, 2) {
                        if move_overlay(b, i).category != dex::Category::Status
                            && b.state.pokemon[m.0 as usize].flags
                                & crate::state::mon_flags::FORCE_SWITCH
                                == 0
                        {
                            b.damage(
                                b.state.pokemon[m.0 as usize].max_hp as f64 / 10.0,
                                Some(m),
                                crate::actions::Attribution::from_move(m, EffectRef::Dex(ID)),
                            );
                        }
                    }
                }
            }
        }
        _ => unreachable!("unexpected lifeorb callback"),
    }
    Relay::Undefined
}
