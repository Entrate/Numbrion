//! Pinned Showdown reactive stat callbacks, data/abilities.ts:2194.
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
pub const ID: EffectId = dex::ABILITY_INTIMIDATE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_INTIMIDATE_ONSTART];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mut h = BattleHost { b, cx };
    react(hook, &mut h)
}
fn react<H: Host>(hook: HookId, h: &mut H) -> Relay {
    match hook {
        dex::HOOK_ABILITY_INTIMIDATE_ONSTART => onstart(h),
        _ => panic!("unexpected intimidate function site"),
    }
}
// data/abilities.ts:2195. PRNG: none directly; nested core calls may draw.
fn onstart<H: Host>(h: &mut H) -> Relay {
    let m = h.mon(0).unwrap();
    let foes = h.foes(m, true);
    let mut activated = false;
    for t in foes.as_slice() {
        if !activated {
            h.log("-ability", m, ID, false, true);
            activated = true;
        }
        if h.volatile(*t, dex::CONDITION_SUBSTITUTE) {
            h.log("-immune", *t, EffectId::NONE, false, false);
        } else {
            h.boost(boosts(&[(0, -1)]), Some(*t), Some(m), None, true, false);
        }
    }
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/intimidate.tsv");

#[cfg(test)]
#[path = "reactivestats/target_tests.rs"]
mod target_tests;
