//! Ports data/abilities.ts:2214 (Intrepid Sword). No direct PRNG draws; the Atk boost runs
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
pub const ID: EffectId = dex::ABILITY_INTREPIDSWORD;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_INTREPIDSWORD_ONSTART];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_INTREPIDSWORD_ONSTART => on_start(b, cx),
        _ => panic!("unexpected intrepidsword hook"),
    }
}

// data/abilities.ts:2215-2219 onStart(pokemon). PRNG: none directly.
// `pokemon.swordBoost` is the Pokemon's SWORD_BOOST flag (battle-long, survives switching).
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = support::mon(b, cx, 0);
    // if (pokemon.swordBoost) return;
    if b.state.pokemon[pokemon.0 as usize].flags & mon_flags::SWORD_BOOST != 0 {
        return Relay::Undefined;
    }
    // pokemon.swordBoost = true;
    b.state.pokemon[pokemon.0 as usize].flags |= mon_flags::SWORD_BOOST;
    // this.boost({ atk: 1 }, pokemon);
    b.boost(
        support::boosts([(Stat::Atk, 1)]),
        Some(pokemon),
        Attribution::DEFAULT,
        false,
        false,
    );
    Relay::Undefined
}
