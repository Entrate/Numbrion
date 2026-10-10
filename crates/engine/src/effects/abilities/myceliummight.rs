//! Ports data/abilities.ts:2795-2811 (Mycelium Might). PRNG: none.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, Category, HookId},
    effects::{HookWaiver, support::move_overlay},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::scratch::move_runtime,
};
pub const ID: EffectId = dex::ABILITY_MYCELIUMMIGHT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_MYCELIUMMIGHT_ONFRACTIONALPRIORITY,
    dex::HOOK_ABILITY_MYCELIUMMIGHT_ONMODIFYMOVE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:2796-2800 onFractionalPriority(priority, pokemon, target, move) (manifest
        // priority -1): `move.category === 'Status'` -> -0.1, else undefined. The runEvent relay
        // starts at 0 (battle-queue.ts:249) and the event args are (priority, pokemon, null, move).
        // PRNG: none.
        dex::HOOK_ABILITY_MYCELIUMMIGHT_ONFRACTIONALPRIORITY => {
            let Some(mv) = cp::live_move_arg(b, cx, 3) else {
                return Relay::Undefined;
            };
            if move_overlay(b, mv).category == Category::Status {
                return Relay::Number(-0.1);
            }
            Relay::Undefined
        }
        // data/abilities.ts:2801-2805 onModifyMove(move): `move.category === 'Status'` ->
        // `move.ignoreAbility = true`. PRNG: none.
        dex::HOOK_ABILITY_MYCELIUMMIGHT_ONMODIFYMOVE => {
            let Some(mv) = cp::live_move_arg(b, cx, 0) else {
                return Relay::Undefined;
            };
            if move_overlay(b, mv).category == Category::Status {
                b.scratch.moves[mv as usize]
                    .as_mut()
                    .expect("released active move")
                    .runtime_flags |= move_runtime::IGNORE_ABILITY;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Mycelium Might hook"),
    }
}
