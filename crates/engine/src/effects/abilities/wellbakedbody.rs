//! Pinned Showdown reactive stat callbacks, data/abilities.ts:5461.
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
pub const ID: EffectId = dex::ABILITY_WELLBAKEDBODY;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_WELLBAKEDBODY_ONTRYHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mut h = BattleHost { b, cx };
    react(hook, &mut h)
}
fn react<H: Host>(hook: HookId, h: &mut H) -> Relay {
    match hook {
        dex::HOOK_ABILITY_WELLBAKEDBODY_ONTRYHIT => ontryhit(h),
        _ => panic!("unexpected wellbakedbody function site"),
    }
}
// data/abilities.ts:5462. PRNG: none directly; nested core calls may draw.
fn ontryhit<H: Host>(h: &mut H) -> Relay {
    let t = h.mon(0).unwrap();
    if Some(t) != h.mon(1) && h.mv(2).move_type == dex::type_id("Fire").unwrap() {
        if !h
            .boost(boosts(&[(1, 2)]), None, None, None, false, false)
            .truthy()
        {
            h.log("-immune", t, ID, true, false);
        }
        return Relay::Null;
    }
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/wellbakedbody.tsv");
