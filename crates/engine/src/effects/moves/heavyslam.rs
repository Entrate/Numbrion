//! Ports data/moves.ts:8533-8572 (Heavy Slam). No PRNG draws; `getWeight` runs a ModifyWeight event.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
#[path = "movecallbacks/mod.rs"]
mod support;
pub const ID: EffectId = dex::MOVE_HEAVYSLAM;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_HEAVYSLAM_BASEPOWERCALLBACK,
    dex::HOOK_MOVE_HEAVYSLAM_ONTRYHIT,
];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_HEAVYSLAM_BASEPOWERCALLBACK => base_power_callback(b, cx),
        dex::HOOK_MOVE_HEAVYSLAM_ONTRYHIT => on_try_hit(b, cx),
        _ => panic!("unexpected Heavy Slam function site"),
    }
}

// data/moves.ts:8537-8554 basePowerCallback(pokemon, target): direct call, args
// [pokemon, target, move]. Source order matters: `target.getWeight()` runs first, then
// `pokemon.getWeight()` (each a ModifyWeight event, then max(1, weight)); the table compares
// `pokemonWeight >= targetWeight * k` for k = 5, 4, 3, 2 -> 120/100/80/60, else 40.
// `this.debug` prints nothing. PRNG: none.
fn base_power_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let target = mon_arg(b, cx, 1);
    let target_weight = b.get_weight(target);
    let pokemon_weight = b.get_weight(pokemon);
    Relay::Number(support::weight_ratio_base_power(
        pokemon_weight,
        target_weight,
    ))
}

// data/moves.ts:8560-8566 onTryHit(target, pokemon, move): only acts for a target holding the
// 'dynamax' volatile (log `-fail|pokemon|Dynamax`, then null). Evidence that the branch is
// unreachable in gen9randomdoublesbattle: side.ts:264 sets `dynamaxUsed = gen !== 8` (true here)
// so canDynamaxNow() (side.ts:311-319) is false and no dynamax choice/volatile can exist, and the
// generated dex has no Dynamax condition. The handler therefore returns `undefined` for every
// target. PRNG: none.
fn on_try_hit<L: LogSink>(_b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    Relay::Undefined
}
