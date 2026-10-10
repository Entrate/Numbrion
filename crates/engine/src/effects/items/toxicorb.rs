//! Ports data/items.ts:6351 (Toxic Orb). No direct PRNG draws; `trySetStatus` runs the
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
pub const ID: EffectId = dex::ITEM_TOXICORB;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_TOXICORB_ONRESIDUAL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ITEM_TOXICORB_ONRESIDUAL => on_residual(b, cx),
        _ => panic!("unexpected toxicorb hook"),
    }
}

// data/items.ts:6351-6353 onResidual(pokemon). PRNG: none directly.
// Residual order 28 / subOrder 3 is manifest metadata. The `[from] item: Toxic Orb` line is
// printed by the toxic condition's onStart (conditions.ts:155-165, shared `log_status`).
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // pokemon.trySetStatus('tox', pokemon);  (sourceEffect null -> setStatus falls back to this.effect)
    b.try_set_status(
        pokemon,
        dex::CONDITION_TOX,
        Attribution::from_move(pokemon, EffectRef::None),
    );
    Relay::Undefined
}
