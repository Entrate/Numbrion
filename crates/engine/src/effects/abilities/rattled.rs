//! Pinned Showdown reactive stat callbacks, data/abilities.ts:3772.
#![allow(unused_imports)]
use crate::{
    Battle,
    actions::Stat,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
    state::{Status, mon_flags},
};
#[path = "reactivestats/host.rs"]
mod host;
use host::{BattleHost, Host, boosts};
pub const ID: EffectId = dex::ABILITY_RATTLED;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_RATTLED_ONDAMAGINGHIT,
    dex::HOOK_ABILITY_RATTLED_ONAFTERBOOST,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mut h = BattleHost { b, cx };
    react(hook, &mut h)
}
fn react<H: Host>(hook: HookId, h: &mut H) -> Relay {
    match hook {
        dex::HOOK_ABILITY_RATTLED_ONDAMAGINGHIT => ondamaginghit(h),
        dex::HOOK_ABILITY_RATTLED_ONAFTERBOOST => onafterboost(h),
        _ => panic!("unexpected rattled function site"),
    }
}
// data/abilities.ts:3773. PRNG: none directly; nested core calls may draw.
fn ondamaginghit<H: Host>(h: &mut H) -> Relay {
    let ty = h.mv(3).move_type;
    if ["Dark", "Bug", "Ghost"]
        .iter()
        .any(|s| ty == dex::type_id(s).unwrap())
    {
        h.boost(boosts(&[(4, 1)]), None, None, None, false, false);
    }
    Relay::Undefined
}
// data/abilities.ts:3778. PRNG: none directly; nested core calls may draw.
fn onafterboost<H: Host>(h: &mut H) -> Relay {
    if h.effect_id(3) == dex::ABILITY_INTIMIDATE {
        let b = h.boost_arg(0);
        if b.present & 1 != 0 && b.values[0] != 0 {
            h.boost(boosts(&[(4, 1)]), None, None, None, false, false);
        }
    }
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/rattled.tsv");
#[cfg(test)]
#[path = "reactivestats/presence_tests.rs"]
mod presence_tests;
