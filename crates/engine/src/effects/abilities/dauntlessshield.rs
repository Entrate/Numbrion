//! Ports data/abilities.ts:853 (Dauntless Shield). No direct PRNG draws; the Def boost runs
//! the usual boost events, whose listeners may shuffle speed ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::{Attribution, Stat},
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::mon_flags,
};
pub const ID: EffectId = dex::ABILITY_DAUNTLESSSHIELD;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_DAUNTLESSSHIELD_ONSTART];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_DAUNTLESSSHIELD_ONSTART => on_start(b, cx),
        _ => panic!("unexpected dauntlessshield hook"),
    }
}

// data/abilities.ts:854-858 onStart(pokemon). PRNG: none directly.
// `pokemon.shieldBoost` is the Pokemon's SHIELD_BOOST flag (battle-long, survives switching).
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = support::mon(b, cx, 0);
    // if (pokemon.shieldBoost) return;
    if b.state.pokemon[pokemon.0 as usize].flags & mon_flags::SHIELD_BOOST != 0 {
        return Relay::Undefined;
    }
    // pokemon.shieldBoost = true;
    b.state.pokemon[pokemon.0 as usize].flags |= mon_flags::SHIELD_BOOST;
    // this.boost({ def: 1 }, pokemon);
    b.boost(
        support::boosts([(Stat::Def, 1)]),
        Some(pokemon),
        Attribution::DEFAULT,
        false,
        false,
    );
    Relay::Undefined
}
