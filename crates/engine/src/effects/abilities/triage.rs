//! Ports data/abilities.ts:5174-5182 (Triage). PRNG: none.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{move_overlay, relay_number},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_TRIAGE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_TRIAGE_ONMODIFYPRIORITY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5175-5177 onModifyPriority(priority, pokemon, target, move):
        // `move?.flags['heal']` -> priority + 3, else undefined. PRNG: none.
        dex::HOOK_ABILITY_TRIAGE_ONMODIFYPRIORITY => {
            let priority = relay_number(b, cx, 0);
            let Some(mv) = cp::live_move_arg(b, cx, 3) else {
                return Relay::Undefined;
            };
            if move_overlay(b, mv).flags & dex::FLAG_HEAL != 0 {
                return Relay::Number(priority + 3.0);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Triage hook"),
    }
}
