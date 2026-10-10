//! Ports data/moves.ts:4784-4804 (Endeavor). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_ENDEAVOR;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_ENDEAVOR_DAMAGECALLBACK,
    dex::HOOK_MOVE_ENDEAVOR_ONTRYIMMUNITY,
];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_ENDEAVOR_DAMAGECALLBACK => damage_callback(b, cx),
        dex::HOOK_MOVE_ENDEAVOR_ONTRYIMMUNITY => on_try_immunity(b, cx),
        _ => panic!("unexpected Endeavor function site"),
    }
}

// data/moves.ts:4788-4790 damageCallback(pokemon, target): direct call from getDamage
// (battle-actions.ts:1575-1577), args [pokemon, target]. `target.getUndynamaxedHP() -
// pokemon.hp` (no Dynamax in this format: the target's current HP); may be zero or negative and
// is returned unclamped. PRNG: none.
fn damage_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let target = mon_arg(b, cx, 1);
    let target_hp = b.get_undynamaxed_hp(target, None);
    Relay::Number(target_hp - f64::from(b.state.pokemon[pokemon.0 as usize].hp))
}

// data/moves.ts:4796-4798 onTryImmunity(target, pokemon): singleEvent('TryImmunity', move, {},
// target, pokemon, move) (battle-actions.ts:669), args [target, pokemon, move].
// `return pokemon.hp < target.hp`: a real boolean (false makes hitStepTryImmunity log `-immune`).
// PRNG: none.
fn on_try_immunity<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let pokemon = mon_arg(b, cx, 1);
    Relay::Bool(b.state.pokemon[pokemon.0 as usize].hp < b.state.pokemon[target.0 as usize].hp)
}
