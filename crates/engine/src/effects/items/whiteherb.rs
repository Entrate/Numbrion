//! Ports data/items.ts:7658 (White Herb). No direct PRNG draws; useItem's events/callbacks
//! (UseItem, Use, AfterUseItem) retain theirs.
//!
//! # Payload map (item state)
//!
//! `this.effectState.boosts` (a `SparseBoostsTable` that only exists while `onStart` runs):
//! presence bit `HERB_BOOSTS_PRESENT` (custom bit 8) plus word 0 = bit per `boosts` index
//! (atk, def, spa, spd, spe, accuracy, evasion) of the keys that were negative. Every value
//! in the table is 0, so the key set is the whole object (`for (i in boosts)` order is the
//! ascending index order in which the Pokemon's own boosts object is created).
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ITEM_WHITEHERB;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_WHITEHERB_FLING_EFFECT,
    dex::HOOK_ITEM_WHITEHERB_ONSTART,
    dex::HOOK_ITEM_WHITEHERB_ONANYSWITCHIN,
    dex::HOOK_ITEM_WHITEHERB_ONANYAFTERMEGA,
    dex::HOOK_ITEM_WHITEHERB_ONANYAFTERMOVE,
    dex::HOOK_ITEM_WHITEHERB_ONRESIDUAL,
    dex::HOOK_ITEM_WHITEHERB_ONUSE,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:7663 fling.effect(pokemon): collect every negative stage as 0; when any
        // exists, pokemon.setBoost(boosts) then `-clearnegativeboost|pokemon|[silent]`.
        // Returns undefined. PRNG: none.
        dex::HOOK_ITEM_WHITEHERB_FLING_EFFECT => {
            let pokemon = support::mon(b, cx, 0);
            let mask = support::negative_boost_mask(b, pokemon);
            if mask != 0 {
                b.set_boost(pokemon, support::zeroed_boosts(mask));
                support::add_clear_negative_boost(b, pokemon);
            }
            Relay::Undefined
        }
        // data/items.ts:7679 onStart(pokemon).
        dex::HOOK_ITEM_WHITEHERB_ONSTART => {
            let pokemon = support::mon(b, cx, 0);
            start(b, cx, pokemon);
            Relay::Undefined
        }
        // data/items.ts:7693,7696,7699 onAnySwitchIn()/onAnyAfterMega()/onAnyAfterMove():
        // `((this.effect as any).onStart).call(this, this.effectState.target)`: a direct call
        // of onStart with the same effect, state and frame, passing the item holder.
        dex::HOOK_ITEM_WHITEHERB_ONANYSWITCHIN
        | dex::HOOK_ITEM_WHITEHERB_ONANYAFTERMEGA
        | dex::HOOK_ITEM_WHITEHERB_ONANYAFTERMOVE => {
            let holder = support::state_target(b, cx);
            start(b, cx, holder);
            Relay::Undefined
        }
        // data/items.ts:7703 onResidual(pokemon) (order 29 is manifest metadata):
        // `.call(this, pokemon)` of onStart.
        dex::HOOK_ITEM_WHITEHERB_ONRESIDUAL => {
            let pokemon = support::mon(b, cx, 0);
            start(b, cx, pokemon);
            Relay::Undefined
        }
        // data/items.ts:7706 onUse(pokemon): `pokemon.setBoost(this.effectState.boosts);
        // this.add('-clearnegativeboost', pokemon, '[silent]');`. The table only exists while
        // onStart is running; with no table `setBoost`'s for-in is a no-op but the line is
        // still logged. Returns undefined. PRNG: none.
        dex::HOOK_ITEM_WHITEHERB_ONUSE => {
            let pokemon = support::mon(b, cx, 0);
            let cell = b.hook_state(cx);
            let mask = if cell.present & support::HERB_BOOSTS_PRESENT != 0 {
                cell.payload.words[0] as u8
            } else {
                0
            };
            b.set_boost(pokemon, support::zeroed_boosts(mask));
            support::add_clear_negative_boost(b, pokemon);
            Relay::Undefined
        }
        _ => panic!("unexpected White Herb hook"),
    }
}

/// data/items.ts:7679-7691 onStart(pokemon), also reached through the direct `.call`s above:
///
/// ```text
/// this.effectState.boosts = {};
/// let ready = false;
/// for (i in pokemon.boosts) if (pokemon.boosts[i] < 0) { ready = true; this.effectState.boosts[i] = 0; }
/// if (ready) (this.effectState.target as Pokemon).useItem();
/// delete this.effectState.boosts;
/// ```
///
/// `pokemon` is the callback argument (the holder in every call form); `useItem` runs on
/// `effectState.target`. The table is published in the item state before `useItem` so that
/// `onUse` (same state cell) sees it, and removed afterwards. Returns undefined. PRNG: none
/// directly; useItem's events may sort ties.
fn start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, pokemon: crate::ids::MonId) {
    let mask = support::negative_boost_mask(b, pokemon);
    {
        let cell = b.hook_state_mut(cx);
        cell.present |= support::HERB_BOOSTS_PRESENT;
        cell.payload.words[0] = u32::from(mask);
    }
    if mask != 0 {
        let holder = support::state_target(b, cx);
        b.use_item(holder, Attribution::DEFAULT);
    }
    // `delete this.effectState.boosts` (the cell may already have been cleared by useItem).
    let cell = b.hook_state_mut(cx);
    cell.present &= !support::HERB_BOOSTS_PRESENT;
    cell.payload.words[0] = 0;
}
