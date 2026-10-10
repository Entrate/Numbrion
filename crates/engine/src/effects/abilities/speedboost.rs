//! Ports data/abilities.ts:4454 (Speed Boost). No direct PRNG draws; the Spe boost runs the
//! usual boost events, whose listeners may shuffle speed ties. Residual order 28 / sub-order 2
//! is manifest metadata.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::{Attribution, Stat},
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_SPEEDBOOST;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SPEEDBOOST_ONRESIDUAL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_SPEEDBOOST_ONRESIDUAL => on_residual(b, cx),
        _ => panic!("unexpected speedboost hook"),
    }
}

// data/abilities.ts:4457-4461 onResidual(pokemon). PRNG: none directly.
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = support::mon(b, cx, 0);
    // if (pokemon.activeTurns)  (only truthiness is observable)
    if b.state.pokemon[pokemon.0 as usize].active_turns != 0 {
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
