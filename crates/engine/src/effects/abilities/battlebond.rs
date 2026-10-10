//! Pinned Showdown reactive stat callbacks, data/abilities.ts:362.
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
pub const ID: EffectId = dex::ABILITY_BATTLEBOND;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_BATTLEBOND_ONSOURCEAFTERFAINT,
    dex::HOOK_ABILITY_BATTLEBOND_ONMODIFYMOVE,
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
        dex::HOOK_ABILITY_BATTLEBOND_ONSOURCEAFTERFAINT => onsourceafterfaint(h),
        dex::HOOK_ABILITY_BATTLEBOND_ONMODIFYMOVE => onmodifymove(h),
        _ => panic!("unexpected battlebond function site"),
    }
}
// data/abilities.ts:363. PRNG: none directly; nested core calls may draw.
fn onsourceafterfaint<H: Host>(h: &mut H) -> Relay {
    let m = h.mon(2).unwrap();
    let p = h.pokemon(m);
    if p.flags & mon_flags::BOND_TRIGGERED != 0 {
        return Relay::Undefined;
    }
    if h.effect(3) != Some(dex::EffectType::Move) {
        return Relay::Undefined;
    }
    if p.species == dex::SPECIES_GRENINJABOND
        && p.hp != 0
        && p.flags & mon_flags::TRANSFORMED == 0
        && h.foe_left(m)
    {
        h.boost(
            boosts(&[(0, 1), (2, 1), (4, 1)]),
            Some(m),
            Some(m),
            Some(ID),
            false,
            false,
        );
        h.log("-activate", m, ID, false, false);
        h.flag(m, mon_flags::BOND_TRIGGERED);
    }
    Relay::Undefined
}
// data/abilities.ts:373. PRNG: none directly; nested core calls may draw.
fn onmodifymove<H: Host>(h: &mut H) -> Relay {
    let m = h.mon(1).unwrap();
    if h.mv(0).key == "watershuriken"
        && h.species_name(m) == "Greninja-Ash"
        && h.pokemon(m).flags & mon_flags::TRANSFORMED == 0
    {
        h.multihit(0, 3);
    }
    Relay::Undefined
}
#[cfg(test)]
#[path = "reactivestats/tests.rs"]
mod tests;

#[cfg(test)]
const VECTORS: &str = include_str!("reactivestats/vectors/battlebond.tsv");
