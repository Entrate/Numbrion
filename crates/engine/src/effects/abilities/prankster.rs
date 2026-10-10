//! Ports data/abilities.ts:3426-3436 (Prankster). PRNG: none.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, Category, HookId},
    effects::{
        HookWaiver,
        support::{move_overlay, relay_number},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::scratch::move_runtime,
};
pub const ID: EffectId = dex::ABILITY_PRANKSTER;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_PRANKSTER_ONMODIFYPRIORITY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3427-3432 onModifyPriority(priority, pokemon, target, move):
        // `move?.category === 'Status'` -> `move.pranksterBoosted = true; return priority + 1`.
        // The flag lives on the queued move's ActiveMove (getActionSpeed copies it back into the
        // queued action and the move frame is rebuilt from it before the move runs). PRNG: none.
        dex::HOOK_ABILITY_PRANKSTER_ONMODIFYPRIORITY => {
            let priority = relay_number(b, cx, 0);
            let Some(mv) = cp::live_move_arg(b, cx, 3) else {
                return Relay::Undefined;
            };
            if move_overlay(b, mv).category != Category::Status {
                return Relay::Undefined;
            }
            b.scratch.moves[mv as usize]
                .as_mut()
                .expect("released active move")
                .runtime_flags |= move_runtime::PRANKSTER_BOOSTED;
            Relay::Number(priority + 1.0)
        }
        _ => panic!("unexpected Prankster hook"),
    }
}
