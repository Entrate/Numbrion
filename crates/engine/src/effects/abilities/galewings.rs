//! Ports data/abilities.ts:1589-1597 (Gale Wings). PRNG: none.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_overlay, relay_number},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_GALEWINGS;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_GALEWINGS_ONMODIFYPRIORITY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1590-1592 onModifyPriority(priority, pokemon, target, move):
        // `move?.type === 'Flying' && pokemon.hp === pokemon.maxhp` -> priority + 1, else undefined.
        // Arguments are the runEvent relay (priority) then (pokemon, target, move). `move.type` is
        // the ActiveMove's current type. PRNG: none.
        dex::HOOK_ABILITY_GALEWINGS_ONMODIFYPRIORITY => {
            let priority = relay_number(b, cx, 0);
            let pokemon = mon_arg(b, cx, 1);
            let Some(mv) = cp::live_move_arg(b, cx, 3) else {
                return Relay::Undefined;
            };
            let p = &b.state.pokemon[pokemon.0 as usize];
            if move_overlay(b, mv).move_type == cp::TYPE_FLYING && p.hp == p.max_hp {
                return Relay::Number(priority + 1.0);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Gale Wings hook"),
    }
}
