//! Pinned Showdown reactive stat callbacks, data/abilities.ts:901.
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
pub const ID: EffectId = dex::ABILITY_DEFIANT;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_DEFIANT_ONAFTEREACHBOOST];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mut h = BattleHost { b, cx };
    react(hook, &mut h)
}
fn react<H: Host>(hook: HookId, h: &mut H) -> Relay {
    match hook {
        dex::HOOK_ABILITY_DEFIANT_ONAFTEREACHBOOST => onaftereachboost(h),
        _ => panic!("unexpected defiant function site"),
    }
}
// data/abilities.ts:902. PRNG: none directly; nested core calls may draw.
fn onaftereachboost<H: Host>(h: &mut H) -> Relay {
    let t = h.mon(1).unwrap();
    let Some(s) = h.mon(2) else {
        return Relay::Undefined;
    };
    if t.side() == s.side() {
        return Relay::Undefined;
    }
    let b = h.boost_arg(0);
    if b.order[..b.len as usize]
        .iter()
        .any(|i| b.values[*i as usize] < 0)
    {
        h.boost(boosts(&[(0, 2)]), Some(t), Some(t), None, false, true);
    }
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/defiant.tsv");
