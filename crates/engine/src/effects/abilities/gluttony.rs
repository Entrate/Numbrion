//! Ports data/abilities.ts:1619 (Gluttony). No PRNG draws.
//!
//! # Payload map (ability state)
//!
//! `pokemon.abilityState.gluttony`: presence bit `GLUTTONY_FLAG` (custom bit 8) of the
//! Pokemon's ability state cell, set means `true`. It is only ever assigned `true` and is
//! dropped with the cell when the ability changes, exactly like the JS object property.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_GLUTTONY;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_GLUTTONY_ONSTART,
    dex::HOOK_ABILITY_GLUTTONY_ONDAMAGE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1620 onStart(pokemon): `pokemon.abilityState.gluttony = true;`
        // Returns undefined. PRNG: none.
        dex::HOOK_ABILITY_GLUTTONY_ONSTART => {
            let pokemon = support::mon(b, cx, 0);
            support::set_gluttony(b, pokemon);
            Relay::Undefined
        }
        // data/abilities.ts:1623 onDamage(item, pokemon): `pokemon.abilityState.gluttony = true;`
        // (the first parameter is really the damage relay; the Pokemon is argument 1).
        // Returns undefined, leaving the damage unchanged. PRNG: none.
        dex::HOOK_ABILITY_GLUTTONY_ONDAMAGE => {
            let pokemon = support::mon(b, cx, 1);
            support::set_gluttony(b, pokemon);
            Relay::Undefined
        }
        _ => panic!("unexpected Gluttony hook"),
    }
}
