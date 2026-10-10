//! Ports data/items.ts:2151 (Flame Orb). No direct PRNG draws; `trySetStatus` runs the
//! SetStatus/Immunity/AfterSetStatus events, whose listeners may speed-sort ties.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ITEM_FLAMEORB;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_FLAMEORB_ONRESIDUAL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_FLAMEORB_ONRESIDUAL => on_residual(b, cx),
        _ => panic!("unexpected flameorb hook"),
    }
}

// data/items.ts:2151-2153 onResidual(pokemon). PRNG: none directly.
// Residual order 28 / subOrder 3 is manifest metadata. The `[from] item: Flame Orb` line is
// printed by the burn condition's onStart (conditions.ts:5-16, shared `log_status`).
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // pokemon.trySetStatus('brn', pokemon);  (sourceEffect null -> setStatus falls back to this.effect)
    b.try_set_status(
        pokemon,
        dex::CONDITION_BRN,
        Attribution::from_move(pokemon, EffectRef::None),
    );
    Relay::Undefined
}
