//! Ports data/items.ts:877 (Chesto Berry). No direct PRNG draws; nested core calls (eatItem,
//! cureStatus) retain event draws.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ITEM_CHESTOBERRY;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_CHESTOBERRY_ONUPDATE,
    dex::HOOK_ITEM_CHESTOBERRY_ONEAT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:885 onUpdate(pokemon): `if (pokemon.status === 'slp') pokemon.eatItem();`
        // Returns undefined. PRNG: none directly.
        dex::HOOK_ITEM_CHESTOBERRY_ONUPDATE => {
            let pokemon = support::mon(b, cx, 0);
            if support::is_asleep(b, pokemon) {
                support::eat_item(b, pokemon);
            }
            Relay::Undefined
        }
        // data/items.ts:890 onEat(pokemon): `if (pokemon.status === 'slp') pokemon.cureStatus();`
        // (silent = false: `-curestatus|mon|slp|[msg]`). Returns undefined. PRNG: none directly.
        dex::HOOK_ITEM_CHESTOBERRY_ONEAT => {
            let pokemon = support::mon(b, cx, 0);
            if support::is_asleep(b, pokemon) {
                b.cure_status(pokemon, false);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Chesto Berry hook"),
    }
}
