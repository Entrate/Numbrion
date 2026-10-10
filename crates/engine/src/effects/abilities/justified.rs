//! Ports data/abilities.ts:2250 (Justified). No direct PRNG draws; the Atk boost runs the
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
pub const ID: EffectId = dex::ABILITY_JUSTIFIED;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_JUSTIFIED_ONDAMAGINGHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_JUSTIFIED_ONDAMAGINGHIT => on_damaging_hit(b, cx),
        _ => panic!("unexpected justified hook"),
    }
}

// data/abilities.ts:2251-2255 onDamagingHit(damage, target, source, move). PRNG: none directly.
fn on_damaging_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // if (move.type === 'Dark')  (the mutable overlay type, after any -ate style change)
    let move_type = move_overlay(b, move_arg(b, cx, 3)).move_type;
    if move_type == support::TYPE_DARK {
        // this.boost({ atk: 1 });
        b.boost(
            support::boosts([(Stat::Atk, 1)]),
            None,
            Attribution::DEFAULT,
            false,
            false,
        );
    }
    Relay::Undefined
}
