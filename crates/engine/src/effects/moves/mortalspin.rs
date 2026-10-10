//! Ports data/moves.ts:12323-12357 (Mortal Spin onAfterHit + onAfterSubDamage; the 100% psn
//! secondary is plain data). Payload: none. No direct PRNG draws; removal events retain their own.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_MORTALSPIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_MORTALSPIN_ONAFTERHIT,
    dex::HOOK_MOVE_MORTALSPIN_ONAFTERSUBDAMAGE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:12323 onAfterHit(target, pokemon, move): same body as Rapid Spin with
        // `[from] move: Mortal Spin`. Returns undefined. PRNG: none.
        dex::HOOK_MOVE_MORTALSPIN_ONAFTERHIT => {
            support::spin_clear(b, cx, dex::MOVE_MORTALSPIN, false)
        }
        // data/moves.ts:12339 onAfterSubDamage(damage, target, pokemon, move): every step gated on
        // `pokemon.hp`. Returns undefined.
        dex::HOOK_MOVE_MORTALSPIN_ONAFTERSUBDAMAGE => {
            support::spin_clear(b, cx, dex::MOVE_MORTALSPIN, true)
        }
        _ => panic!("unexpected Mortal Spin function site"),
    }
}
