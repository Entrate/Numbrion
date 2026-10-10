//! Ports data/moves.ts:14703-14737 (Rapid Spin onAfterHit + onAfterSubDamage; the +1 Spe secondary
//! is plain data). Payload: none. No direct PRNG draws; removal events retain their own.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_RAPIDSPIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_RAPIDSPIN_ONAFTERHIT,
    dex::HOOK_MOVE_RAPIDSPIN_ONAFTERSUBDAMAGE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:14703 onAfterHit(target, pokemon, move): unless move.hasSheerForce, in order:
        // removeVolatile('leechseed') -> `-end|mon|Leech Seed|[from] move: Rapid Spin|[of] mon`;
        // each of spikes/toxicspikes/stealthrock/stickyweb (+ gmaxsteelsurge, never present here)
        // removed from the user's side -> `-sideend|side|<Name>|[from] move: Rapid Spin|[of] mon`;
        // then removeVolatile('partiallytrapped'). Returns undefined. PRNG: none.
        dex::HOOK_MOVE_RAPIDSPIN_ONAFTERHIT => {
            support::spin_clear(b, cx, dex::MOVE_RAPIDSPIN, false)
        }
        // data/moves.ts:14719 onAfterSubDamage(damage, target, pokemon, move): same body, but every
        // step is additionally gated on `pokemon.hp`. Returns undefined.
        dex::HOOK_MOVE_RAPIDSPIN_ONAFTERSUBDAMAGE => {
            support::spin_clear(b, cx, dex::MOVE_RAPIDSPIN, true)
        }
        _ => panic!("unexpected Rapid Spin function site"),
    }
}
