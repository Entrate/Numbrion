//! Pinned Showdown reactive stat callbacks, data/abilities.ts:414.
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
pub const ID: EffectId = dex::ABILITY_BERSERK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_BERSERK_ONDAMAGE,
    dex::HOOK_ABILITY_BERSERK_ONTRYEATITEM,
    dex::HOOK_ABILITY_BERSERK_ONAFTERMOVESECONDARY,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
// checkedBerserk: custom presence bit 8, payload word 0 boolean; absent differs from false.
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mut h = BattleHost { b, cx };
    react(hook, &mut h)
}
fn react<H: Host>(hook: HookId, h: &mut H) -> Relay {
    match hook {
        dex::HOOK_ABILITY_BERSERK_ONDAMAGE => ondamage(h),
        dex::HOOK_ABILITY_BERSERK_ONTRYEATITEM => ontryeatitem(h),
        dex::HOOK_ABILITY_BERSERK_ONAFTERMOVESECONDARY => onaftermovesecondary(h),
        _ => panic!("unexpected berserk function site"),
    }
}
// data/abilities.ts:415. PRNG: none directly; nested core calls may draw.
fn ondamage<H: Host>(h: &mut H) -> Relay {
    let checked = !(h.effect(3) == Some(dex::EffectType::Move)
        && !h.mv(3).multihit
        && !h.suppress_secondaries());
    h.set_checked(checked);
    Relay::Undefined
}
// data/abilities.ts:421. PRNG: none directly; nested core calls may draw.
fn ontryeatitem<H: Host>(h: &mut H) -> Relay {
    let id = h.effect_id(0);
    let healing = [
        "aguavberry",
        "enigmaberry",
        "figyberry",
        "iapapaberry",
        "magoberry",
        "sitrusberry",
        "wikiberry",
        "oranberry",
        "berryjuice",
    ];
    if healing.iter().any(|s| dex::effect(id).key == *s) {
        return h.checked().map_or(Relay::Undefined, Relay::Bool);
    }
    Relay::Bool(true)
}
// data/abilities.ts:430. PRNG: none directly; nested core calls may draw.
fn onaftermovesecondary<H: Host>(h: &mut H) -> Relay {
    h.set_checked(true);
    let t = h.mon(0).unwrap();
    let Some(s) = h.mon(1) else {
        return Relay::Undefined;
    };
    if s == t || h.pokemon(t).hp == 0 || h.mv(2).total_damage == 0 {
        return Relay::Undefined;
    }
    let Some(last) = h.last_damage(t) else {
        return Relay::Undefined;
    };
    let mv = h.mv(2);
    let damage = if mv.multihit && !mv.smart {
        mv.total_damage as f64
    } else {
        last
    };
    let p = h.pokemon(t);
    if p.hp as f64 <= p.max_hp as f64 / 2.0 && p.hp as f64 + damage > p.max_hp as f64 / 2.0 {
        h.boost(boosts(&[(2, 1)]), Some(t), Some(t), None, false, false);
    }
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/berserk.tsv");
#[cfg(test)]
#[path = "reactivestats/state_tests.rs"]
mod state_tests;
