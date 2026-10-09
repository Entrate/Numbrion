//! Ports data/items.ts:1064: complete scoped callbacks; no direct PRNG draws.
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
pub const ID: EffectId = dex::ITEM_CLEARAMULET;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_CLEARAMULET_ONTRYBOOST];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_CLEARAMULET_ONTRYBOOST => {
            let target = mon_arg(b, cx, 1);
            if Battle::<L>::arg_mon(b.event_arg(cx, 2)) == Some(target) {
                return Relay::Undefined;
            }
            let EventArg::Relay(Relay::Boosts(i)) = b.event_arg(cx, 0) else {
                panic!("Clear Amulet requires boost object")
            };
            let boosts = b.scratch_boosts(i);
            let mut removed = false;
            let mut len = 0;
            for j in 0..boosts.len as usize {
                let stat = boosts.order[j];
                if boosts.values[stat as usize] < 0 {
                    boosts.present &= !(1 << stat);
                    boosts.values[stat as usize] = 0;
                    removed = true;
                } else {
                    boosts.order[len] = stat;
                    len += 1;
                }
            }
            boosts.len = len as u8;
            boosts.order[len..].fill(0);
            let effect = match b.event_arg(cx, 3) {
                EventArg::Effect(e) => e,
                EventArg::Move(i) => EffectRef::ActiveMove(i),
                _ => EffectRef::None,
            };
            let secondaries = match effect {
                EffectRef::ActiveMove(i) => move_overlay(b, i).secondaries_present,
                _ => dex::effect(b.event_effect_id(effect))
                    .data
                    .get(dex::FIELD_SECONDARIES)
                    .is_some(),
            };
            if removed && !secondaries {
                // Octolock is absent from this Gen 9 scope.
                b.add(crate::log::LogEntry::new(
                    "-fail",
                    &[
                        crate::log::LogArg::Mon(target),
                        crate::log::LogArg::Text("unboost"),
                    ],
                    &[
                        crate::log::LogTag::From(EffectRef::Dex(ID)),
                        crate::log::LogTag::Of(target),
                    ],
                ));
            }
        }
        _ => unreachable!("unexpected clearamulet callback"),
    }
    Relay::Undefined
}
