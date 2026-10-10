//! Ports data/items.ts:5744 (Sitrus Berry). No direct PRNG draws; nested core calls (eatItem,
//! TryHeal, heal) retain event draws.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ITEM_SITRUSBERRY;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_SITRUSBERRY_ONUPDATE,
    dex::HOOK_ITEM_SITRUSBERRY_ONTRYEATITEM,
    dex::HOOK_ITEM_SITRUSBERRY_ONEAT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:5752 onUpdate(pokemon): `if (pokemon.hp <= pokemon.maxhp / 2)
        // pokemon.eatItem();` (no Gluttony clause). Returns undefined. PRNG: none directly.
        dex::HOOK_ITEM_SITRUSBERRY_ONUPDATE => {
            let pokemon = support::mon(b, cx, 0);
            if support::hp_at_most_fraction(b, pokemon, 2.0) {
                support::eat_item(b, pokemon);
            }
            Relay::Undefined
        }
        // data/items.ts:5757 onTryEatItem(item, pokemon): `if (!this.runEvent('TryHeal', pokemon,
        // null, this.effect, pokemon.baseMaxhp / 4)) return false;` The pokemon is argument 1
        // (argument 0 is the item relay). PRNG: none directly.
        dex::HOOK_ITEM_SITRUSBERRY_ONTRYEATITEM => {
            let pokemon = support::mon(b, cx, 1);
            support::try_eat_heal(b, pokemon, 4.0)
        }
        // data/items.ts:5760 onEat(pokemon): `this.heal(pokemon.baseMaxhp / 4);` with target,
        // source and effect defaulted from the Eat event. Returns undefined. PRNG: none directly.
        dex::HOOK_ITEM_SITRUSBERRY_ONEAT => {
            let pokemon = support::mon(b, cx, 0);
            support::heal_base_max_hp(b, pokemon, 4.0);
            Relay::Undefined
        }
        _ => panic!("unexpected Sitrus Berry hook"),
    }
}
