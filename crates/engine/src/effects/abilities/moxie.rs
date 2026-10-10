//! Pinned Showdown reactive stat callbacks, data/abilities.ts:2750.
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
pub const ID: EffectId = dex::ABILITY_MOXIE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_MOXIE_ONSOURCEAFTERFAINT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mut h = BattleHost { b, cx };
    react(hook, &mut h)
}
fn react<H: Host>(hook: HookId, h: &mut H) -> Relay {
    match hook {
        dex::HOOK_ABILITY_MOXIE_ONSOURCEAFTERFAINT => onsourceafterfaint(h),
        _ => panic!("unexpected moxie function site"),
    }
}
// data/abilities.ts:2751. PRNG: none directly; nested core calls may draw.
fn onsourceafterfaint<H: Host>(h: &mut H) -> Relay {
    if h.effect(3) == Some(dex::EffectType::Move) {
        let n = h.number(0) as i8;
        let m = h.mon(2).unwrap();
        h.boost(boosts(&[(0, n)]), Some(m), None, None, false, false);
    }
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/moxie.tsv");
