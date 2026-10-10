//! Ports data/items.ts:6309 (Throat Spray). No direct PRNG draws; useItem's events/callbacks
//! (UseItem, boost, Use, AfterUseItem) retain theirs.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ITEM_THROATSPRAY;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_THROATSPRAY_ONAFTERMOVESECONDARYSELF];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:6315 onAfterMoveSecondarySelf(target, source, move):
        //   if (move.flags['sound']) target.useItem();
        // Argument 0 is the move's user (named `target` here); the item's `boosts: { spa: 1 }`
        // are applied by useItem itself (pokemon.ts:1811-1849). Returns undefined.
        // PRNG: none directly.
        dex::HOOK_ITEM_THROATSPRAY_ONAFTERMOVESECONDARYSELF => {
            let user = support::mon(b, cx, 0);
            let sound = move_overlay(b, move_arg(b, cx, 2)).flags & dex::FLAG_SOUND != 0;
            if sound {
                b.use_item(user, Attribution::DEFAULT);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Throat Spray hook"),
    }
}
