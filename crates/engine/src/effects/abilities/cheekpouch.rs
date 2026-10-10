//! Ports data/abilities.ts:492 (Cheek Pouch). No direct PRNG draws; heal's events retain theirs.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_CHEEKPOUCH;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_CHEEKPOUCH_ONEATITEM];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:493 onEatItem(item, pokemon): `this.heal(pokemon.baseMaxhp / 3);`
        // The pokemon is argument 1 (argument 0 is the item relay); heal's target, source and
        // effect all default from the EatItem event / this.effect (Cheek Pouch itself).
        // Returns undefined. PRNG: none directly.
        dex::HOOK_ABILITY_CHEEKPOUCH_ONEATITEM => {
            let pokemon = support::mon(b, cx, 1);
            support::heal_base_max_hp(b, pokemon, 3.0);
            Relay::Undefined
        }
        _ => panic!("unexpected Cheek Pouch hook"),
    }
}
