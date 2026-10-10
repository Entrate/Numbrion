//! Ports data/abilities.ts:501 (Chilling Neigh). No direct PRNG draws; the Atk boost runs
//! the usual boost events, whose listeners may shuffle speed ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::{Attribution, Stat},
    dex::{self, HookId},
    effects::{HookWaiver, support::relay_number},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_CHILLINGNEIGH;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_CHILLINGNEIGH_ONSOURCEAFTERFAINT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_CHILLINGNEIGH_ONSOURCEAFTERFAINT => on_source_after_faint(b, cx),
        _ => panic!("unexpected chillingneigh hook"),
    }
}

// data/abilities.ts:502-506 onSourceAfterFaint(length, target, source, effect).
// PRNG: none directly.
fn on_source_after_faint<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // if (effect && effect.effectType === 'Move')
    let effect = support::effect_at(b, cx, 3);
    if support::effect_is_move(b, effect) {
        let length = relay_number(b, cx, 0);
        let source = support::mon(b, cx, 2);
        // this.boost({ atk: length }, source);  (source and effect default from the event)
        b.boost(
            support::boosts([(Stat::Atk, length as i8)]),
            Some(source),
            Attribution::DEFAULT,
            false,
            false,
        );
    }
    Relay::Undefined
}
