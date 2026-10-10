//! Pinned Showdown reactive stat callbacks, data/abilities.ts:4992.
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
pub const ID: EffectId = dex::ABILITY_THERMALEXCHANGE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_THERMALEXCHANGE_ONDAMAGINGHIT,
    dex::HOOK_ABILITY_THERMALEXCHANGE_ONUPDATE,
    dex::HOOK_ABILITY_THERMALEXCHANGE_ONSETSTATUS,
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
        dex::HOOK_ABILITY_THERMALEXCHANGE_ONDAMAGINGHIT => ondamaginghit(h),
        dex::HOOK_ABILITY_THERMALEXCHANGE_ONUPDATE => onupdate(h),
        dex::HOOK_ABILITY_THERMALEXCHANGE_ONSETSTATUS => onsetstatus(h),
        _ => panic!("unexpected thermalexchange function site"),
    }
}
// data/abilities.ts:4993. PRNG: none directly; nested core calls may draw.
fn ondamaginghit<H: Host>(h: &mut H) -> Relay {
    if h.mv(3).move_type == dex::type_id("Fire").unwrap() {
        h.boost(boosts(&[(0, 1)]), None, None, None, false, false);
    }
    Relay::Undefined
}
// data/abilities.ts:4998. PRNG: none directly; nested core calls may draw.
fn onupdate<H: Host>(h: &mut H) -> Relay {
    let m = h.mon(0).unwrap();
    if h.pokemon(m).status == Status::Burn {
        h.log("-activate", m, ID, false, false);
        h.cure(m);
    }
    Relay::Undefined
}
// data/abilities.ts:5004. PRNG: none directly; nested core calls may draw.
fn onsetstatus<H: Host>(h: &mut H) -> Relay {
    if h.effect_id(0) != dex::CONDITION_BRN {
        return Relay::Undefined;
    }
    if h.effect_status(3) {
        let t = h.mon(1).unwrap();
        h.log("-immune", t, ID, true, false);
    }
    Relay::Bool(false)
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/thermalexchange.tsv");
#[cfg(test)]
#[path = "reactivestats/log_tests.rs"]
mod log_tests;
