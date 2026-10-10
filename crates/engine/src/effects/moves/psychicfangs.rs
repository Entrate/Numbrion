//! Ports data/moves.ts:14069-14074 (Psychic Fangs onTryHit). Payload: none. No PRNG draws.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_PSYCHICFANGS;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_PSYCHICFANGS_ONTRYHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:14069 onTryHit(pokemon): identical to Brick Break; arg 0 is the event target.
        // Removes Reflect, Light Screen, Aurora Veil from the target's side. Returns undefined.
        // PRNG: none.
        dex::HOOK_MOVE_PSYCHICFANGS_ONTRYHIT => support::shatter_screens(b, cx),
        _ => panic!("unexpected Psychic Fangs function site"),
    }
}
