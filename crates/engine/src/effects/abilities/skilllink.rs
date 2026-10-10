//! Ports data/abilities.ts:4298-4311 (Skill Link). PRNG: none.
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
pub const ID: EffectId = dex::ABILITY_SKILLLINK;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SKILLLINK_ONMODIFYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4299-4306 onModifyMove(move):
        //   if (move.multihit && Array.isArray(move.multihit) && move.multihit.length)
        //     move.multihit = move.multihit[1];            // [min, max] range -> the fixed maximum
        //   if (move.multiaccuracy) delete move.multiaccuracy;
        // ActiveMove model: a range is MULTIHIT_PRESENT | MULTIHIT_RANGE with `multihit = [min, max]`;
        // a fixed count is MULTIHIT_PRESENT without RANGE (`multihit = [n, n]`), so assigning the
        // max leaves a present, non-range count. MULTIACCURACY is the live (not deleted) bit.
        // PRNG: none.
        dex::HOOK_ABILITY_SKILLLINK_ONMODIFYMOVE => {
            let Some(mv) = cp::live_move_arg(b, cx, 0) else {
                return Relay::Undefined;
            };
            let m = b.scratch.moves[mv as usize]
                .as_mut()
                .expect("released active move");
            if m.runtime_flags & move_runtime::MULTIHIT_PRESENT != 0
                && m.runtime_flags & move_runtime::MULTIHIT_RANGE != 0
            {
                m.multihit = [m.multihit[1], m.multihit[1]];
                m.runtime_flags &= !move_runtime::MULTIHIT_RANGE;
            }
            m.runtime_flags &= !move_runtime::MULTIACCURACY;
            Relay::Undefined
        }
        _ => panic!("unexpected Skill Link hook"),
    }
}
