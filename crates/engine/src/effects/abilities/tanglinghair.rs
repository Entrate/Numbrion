//! Pinned Showdown reactive stat callbacks, data/abilities.ts:4906.
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
pub const ID: EffectId = dex::ABILITY_TANGLINGHAIR;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_TANGLINGHAIR_ONDAMAGINGHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mut h = BattleHost { b, cx };
    react(hook, &mut h)
}
fn react<H: Host>(hook: HookId, h: &mut H) -> Relay {
    match hook {
        dex::HOOK_ABILITY_TANGLINGHAIR_ONDAMAGINGHIT => ondamaginghit(h),
        _ => panic!("unexpected tanglinghair function site"),
    }
}
// data/abilities.ts:4907. PRNG: none directly; nested core calls may draw.
fn ondamaginghit<H: Host>(h: &mut H) -> Relay {
    let t = h.mon(1).unwrap();
    let s = h.mon(2).unwrap();
    if h.contact(s, t) {
        h.log("-ability", t, ID, false, false);
        h.boost(boosts(&[(4, -1)]), Some(s), Some(t), None, true, false);
    }
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/tanglinghair.tsv");
