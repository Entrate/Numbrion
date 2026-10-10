//! Ports data/moves.ts:13336-13352 (Photon Geyser). No PRNG draws; `getStat` runs ModifyBoost events.
use crate::{
    Battle,
    actions::{MoveHandle, Stat, StatOptions},
    dex::{self, Category, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_PHOTONGEYSER;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_PHOTONGEYSER_ONMODIFYMOVE];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_PHOTONGEYSER_ONMODIFYMOVE => on_modify_move(b, cx),
        _ => panic!("unexpected Photon Geyser function site"),
    }
}

// data/moves.ts:13345-13347 onModifyMove(move, pokemon): singleEvent('ModifyMove', move, null,
// pokemon, target, move, move) (battle-actions.ts:431), args [move, pokemon, target].
//   if (pokemon.getStat('atk', false, true) > pokemon.getStat('spa', false, true))
//     move.category = 'Physical';
// getStat(unboosted = false, unmodified = true): boosts apply, stat modifier events do not.
// Returns undefined. PRNG: none.
fn on_modify_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let handle = MoveHandle(move_arg(b, cx, 0));
    let pokemon = mon_arg(b, cx, 1);
    let options = StatOptions {
        unboosted: false,
        unmodified: true,
    };
    let atk = b.get_stat(pokemon, Stat::Atk, options);
    let spa = b.get_stat(pokemon, Stat::SpA, options);
    if atk > spa {
        b.active_move_mut(handle).category = Category::Physical;
    }
    Relay::Undefined
}
