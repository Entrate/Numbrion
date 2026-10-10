//! Ports data/abilities.ts:4427 (Soul-Heart). No direct PRNG draws; the Sp. Atk boost runs
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
};
pub const ID: EffectId = dex::ABILITY_SOULHEART;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SOULHEART_ONANYFAINT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_SOULHEART_ONANYFAINT => on_any_faint(b, cx),
        _ => panic!("unexpected soulheart hook"),
    }
}

// data/abilities.ts:4429-4431 onAnyFaint() (priority 1). PRNG: none directly.
fn on_any_faint<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // this.boost({ spa: 1 }, this.effectState.target);
    let holder = support::owner(b, cx);
    b.boost(
        support::boosts([(Stat::SpA, 1)]),
        Some(holder),
        Attribution::DEFAULT,
        false,
        false,
    );
    Relay::Undefined
}
