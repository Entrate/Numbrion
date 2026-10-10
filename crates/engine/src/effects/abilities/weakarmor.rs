//! Ports data/abilities.ts:5450 (Weak Armor). No direct PRNG draws; the Def/Spe change runs
//! the usual boost events, whose listeners may shuffle speed ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::Stat,
    dex::{self, Category, HookId},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_WEAKARMOR;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_WEAKARMOR_ONDAMAGINGHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_WEAKARMOR_ONDAMAGINGHIT => on_damaging_hit(b, cx),
        _ => panic!("unexpected weakarmor hook"),
    }
}

// data/abilities.ts:5451-5455 onDamagingHit(damage, target, source, move). PRNG: none directly.
fn on_damaging_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = support::mon(b, cx, 1);
    // if (move.category === 'Physical')
    if move_overlay(b, move_arg(b, cx, 3)).category == Category::Physical {
        // this.boost({ def: -1, spe: 2 }, target, target);  (keys applied in this order)
        b.boost(
            support::boosts([(Stat::Def, -1), (Stat::Spe, 2)]),
            Some(target),
            support::by(target),
            false,
            false,
        );
    }
    Relay::Undefined
}
