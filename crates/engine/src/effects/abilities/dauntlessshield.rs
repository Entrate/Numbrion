//! Pinned Showdown reactive stat callbacks, data/abilities.ts:853.
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
pub const ID: EffectId = dex::ABILITY_DAUNTLESSSHIELD;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_DAUNTLESSSHIELD_ONSTART];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mut h = BattleHost { b, cx };
    react(hook, &mut h)
}
fn react<H: Host>(hook: HookId, h: &mut H) -> Relay {
    match hook {
        dex::HOOK_ABILITY_DAUNTLESSSHIELD_ONSTART => onstart(h),
        _ => panic!("unexpected dauntlessshield function site"),
    }
}
// data/abilities.ts:854. PRNG: none directly; nested core calls may draw.
fn onstart<H: Host>(h: &mut H) -> Relay {
    let m = h.mon(0).unwrap();
    if h.pokemon(m).flags & mon_flags::SHIELD_BOOST != 0 {
        return Relay::Undefined;
    }
    h.flag(m, mon_flags::SHIELD_BOOST);
    h.boost(boosts(&[(1, 1)]), Some(m), None, None, false, false);
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/dauntlessshield.tsv");
