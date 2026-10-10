//! Ports data/abilities.ts:645 (Competitive). No direct PRNG draws; the Sp. Atk boost runs
//! the usual boost events, whose listeners may shuffle speed ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::Stat,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_COMPETITIVE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_COMPETITIVE_ONAFTEREACHBOOST];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_COMPETITIVE_ONAFTEREACHBOOST => on_after_each_boost(b, cx),
        _ => panic!("unexpected competitive hook"),
    }
}

// data/abilities.ts:646-660 onAfterEachBoost(boost, target, source, effect).
// `boost` is the single-key currentBoost object. PRNG: none directly.
fn on_after_each_boost<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = support::mon(b, cx, 1);
    // if (!source || target.isAlly(source)) return;
    let Some(source) = support::mon_opt(b, cx, 2) else {
        return Relay::Undefined;
    };
    if b.is_ally(target, Some(source)) {
        return Relay::Undefined;
    }
    // for (i in boost) if (boost[i] < 0) statsLowered = true;
    let boost = support::boost_arg(b, cx, 0);
    if support::any_lowered(&boost) {
        // this.boost({ spa: 2 }, target, target, null, false, true);
        b.boost(
            support::boosts([(Stat::SpA, 2)]),
            Some(target),
            support::by(target),
            false,
            true,
        );
    }
    Relay::Undefined
}
