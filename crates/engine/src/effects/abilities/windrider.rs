//! Pinned Showdown reactive stat callbacks, data/abilities.ts:5530.
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
pub const ID: EffectId = dex::ABILITY_WINDRIDER;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_WINDRIDER_ONSTART,
    dex::HOOK_ABILITY_WINDRIDER_ONTRYHIT,
    dex::HOOK_ABILITY_WINDRIDER_ONSIDECONDITIONSTART,
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
        dex::HOOK_ABILITY_WINDRIDER_ONSTART => onstart(h),
        dex::HOOK_ABILITY_WINDRIDER_ONTRYHIT => ontryhit(h),
        dex::HOOK_ABILITY_WINDRIDER_ONSIDECONDITIONSTART => onsideconditionstart(h),
        _ => panic!("unexpected windrider function site"),
    }
}
// data/abilities.ts:5531. PRNG: none directly; nested core calls may draw.
fn onstart<H: Host>(h: &mut H) -> Relay {
    let m = h.mon(0).unwrap();
    if h.tailwind(m.side()) {
        h.boost(boosts(&[(0, 1)]), Some(m), Some(m), None, false, false);
    }
    Relay::Undefined
}
// data/abilities.ts:5536. PRNG: none directly; nested core calls may draw.
fn ontryhit<H: Host>(h: &mut H) -> Relay {
    let t = h.mon(0).unwrap();
    if Some(t) != h.mon(1) && h.mv(2).flags & dex::FLAG_WIND != 0 {
        if !h
            .boost(boosts(&[(0, 1)]), Some(t), Some(t), None, false, false)
            .truthy()
        {
            h.log("-immune", t, ID, true, false);
        }
        return Relay::Null;
    }
    Relay::Undefined
}
// data/abilities.ts:5544. PRNG: none directly; nested core calls may draw.
fn onsideconditionstart<H: Host>(h: &mut H) -> Relay {
    let m = h.owner();
    if h.effect_id(2) == dex::CONDITION_TAILWIND {
        h.boost(boosts(&[(0, 1)]), Some(m), Some(m), None, false, false);
    }
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/windrider.tsv");
