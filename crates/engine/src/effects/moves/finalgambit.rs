//! Ports data/moves.ts:5301-5320 (Final Gambit). No PRNG draws.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_FINALGAMBIT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_FINALGAMBIT_DAMAGECALLBACK];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_FINALGAMBIT_DAMAGECALLBACK => damage_callback(b, cx),
        _ => panic!("unexpected Final Gambit function site"),
    }
}

// data/moves.ts:5305-5309 damageCallback(pokemon): direct call from getDamage
// (battle-actions.ts:1595), args [pokemon, target].
//   const damage = pokemon.hp;
//   pokemon.faint();      // faint(source = null, effect = null): queues the faint, hp -> 0
//   return damage;
// The `selfdestruct: "ifHit"` data field is applied later by the core hit loop. PRNG: none.
fn damage_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let damage = f64::from(b.state.pokemon[pokemon.0 as usize].hp);
    b.faint_pokemon(pokemon, Attribution::NONE);
    Relay::Number(damage)
}
