//! Ports data/abilities.ts:3772 (Rattled). No direct PRNG draws; the Spe boosts run the
//! usual boost events, whose listeners may shuffle speed ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::{Attribution, Stat},
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_RATTLED;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_RATTLED_ONDAMAGINGHIT,
    dex::HOOK_ABILITY_RATTLED_ONAFTERBOOST,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_RATTLED_ONDAMAGINGHIT => on_damaging_hit(b, cx),
        dex::HOOK_ABILITY_RATTLED_ONAFTERBOOST => on_after_boost(b, cx),
        _ => panic!("unexpected rattled hook"),
    }
}

// data/abilities.ts:3773-3777 onDamagingHit(damage, target, source, move). PRNG: none directly.
fn on_damaging_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // if (['Dark', 'Bug', 'Ghost'].includes(move.type))
    let t = move_overlay(b, move_arg(b, cx, 3)).move_type;
    if t == support::TYPE_DARK || t == support::TYPE_BUG || t == support::TYPE_GHOST {
        // this.boost({ spe: 1 });
        b.boost(
            support::boosts([(Stat::Spe, 1)]),
            None,
            Attribution::DEFAULT,
            false,
            false,
        );
    }
    Relay::Undefined
}

// data/abilities.ts:3778-3782 onAfterBoost(boost, target, source, effect). PRNG: none directly.
// `boost` is the whole (post-TryBoost) boost object of the finished boost() call.
fn on_after_boost<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // effect?.name === 'Intimidate' && boost.atk  (only the ability is named Intimidate)
    let effect = support::effect_at(b, cx, 3);
    if b.event_effect_id(effect) == dex::ABILITY_INTIMIDATE {
        let boost = support::boost_arg(b, cx, 0);
        // boost.atk is truthy: present and non-zero
        if support::boost_get(&boost, Stat::Atk).is_some_and(|n| n != 0) {
            // this.boost({ spe: 1 });
            b.boost(
                support::boosts([(Stat::Spe, 1)]),
                None,
                Attribution::DEFAULT,
                false,
                false,
            );
        }
    }
    Relay::Undefined
}
