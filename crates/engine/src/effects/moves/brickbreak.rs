//! Ports data/moves.ts:1831-1836 (Brick Break onTryHit). Payload: none. No PRNG draws.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_BRICKBREAK;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_BRICKBREAK_ONTRYHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:1831 onTryHit(pokemon): handler arg 0 is the event *target* (singleEvent
        // 'TryHit', moveData, {}, target, pokemon, move; battle-actions.ts:1035). Removes Reflect,
        // Light Screen, Aurora Veil from the target's side in that order (each prints `-sideend`),
        // even behind a Substitute. Returns undefined. PRNG: none.
        dex::HOOK_MOVE_BRICKBREAK_ONTRYHIT => support::shatter_screens(b, cx),
        _ => panic!("unexpected Brick Break function site"),
    }
}
