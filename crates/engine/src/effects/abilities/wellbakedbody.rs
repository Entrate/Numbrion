//! Ports data/abilities.ts:5461 (Well-Baked Body). No direct PRNG draws; the Def boost runs
//! the usual boost events, whose listeners may shuffle speed ties.
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
pub const ID: EffectId = dex::ABILITY_WELLBAKEDBODY;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_WELLBAKEDBODY_ONTRYHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_WELLBAKEDBODY_ONTRYHIT => on_try_hit(b, cx),
        _ => panic!("unexpected wellbakedbody hook"),
    }
}

// data/abilities.ts:5462-5469 onTryHit(target, source, move). PRNG: none directly.
fn on_try_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = support::mon(b, cx, 0);
    let source = support::mon_opt(b, cx, 1);
    let move_type = move_overlay(b, move_arg(b, cx, 2)).move_type;
    // if (target !== source && move.type === 'Fire')
    if Some(target) != source && move_type == support::TYPE_FIRE {
        // if (!this.boost({ def: 2 })) this.add('-immune', target, '[from] ability: Well-Baked Body');
        let boosted = b.boost(
            support::boosts([(Stat::Def, 2)]),
            None,
            Attribution::DEFAULT,
            false,
            false,
        );
        if !boosted.truthy() {
            support::add_immune_from(b, target, ID);
        }
        // return null;
        return Relay::Null;
    }
    Relay::Undefined
}
