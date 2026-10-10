//! Pinned Showdown reactive stat callbacks, data/abilities.ts:384.
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
pub const ID: EffectId = dex::ABILITY_BEADSOFRUIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_BEADSOFRUIN_ONSTART,
    dex::HOOK_ABILITY_BEADSOFRUIN_ONANYMODIFYSPD,
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
        dex::HOOK_ABILITY_BEADSOFRUIN_ONSTART => onstart(h),
        dex::HOOK_ABILITY_BEADSOFRUIN_ONANYMODIFYSPD => onanymodifyspd(h),
        _ => panic!("unexpected beadsofruin function site"),
    }
}
// data/abilities.ts:385. PRNG: none directly; nested core calls may draw.
fn onstart<H: Host>(h: &mut H) -> Relay {
    let m = h.mon(0).unwrap();
    if h.suppress_ability(m) {
        return Relay::Undefined;
    }
    h.log("-ability", m, ID, false, false);
    Relay::Undefined
}
// data/abilities.ts:389. PRNG: none directly; nested core calls may draw.
fn onanymodifyspd<H: Host>(h: &mut H) -> Relay {
    let holder = h.owner();
    let m = h.mon(1).unwrap();
    if h.ability(m, ID) {
        return Relay::Undefined;
    }
    let old = h.mv(3).ruined[3];
    if old.is_none() || !h.ability(old.unwrap(), ID) {
        h.ruin(3, 3, holder);
    }
    if h.mv(3).ruined[3] != Some(holder) {
        return Relay::Undefined;
    }
    h.chain();
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/beadsofruin.tsv");

#[cfg(test)]
#[path = "reactivestats/adapter.rs"]
mod adapter;
