//! Ports data/items.ts:3454-3468 (Loaded Dice). The extra-hit part is core code in
//! hitStepMoveHitLoop (battle-actions.ts, "partially implemented" per the item's comment). PRNG: none.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::scratch::move_runtime,
};
pub const ID: EffectId = dex::ITEM_LOADEDDICE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_LOADEDDICE_ONMODIFYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:3460-3464 onModifyMove(move): `if (move.multiaccuracy) delete move.multiaccuracy`
        // (MULTIACCURACY is the live, not-deleted bit). PRNG: none.
        dex::HOOK_ITEM_LOADEDDICE_ONMODIFYMOVE => {
            let Some(mv) = cp::live_move_arg(b, cx, 0) else {
                return Relay::Undefined;
            };
            b.scratch.moves[mv as usize]
                .as_mut()
                .expect("released active move")
                .runtime_flags &= !move_runtime::MULTIACCURACY;
            Relay::Undefined
        }
        _ => panic!("unexpected Loaded Dice hook"),
    }
}
