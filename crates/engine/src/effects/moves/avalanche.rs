//! Ports data/moves.ts:908-930 (Avalanche). No PRNG draws and no nested events.
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
pub const ID: EffectId = dex::MOVE_AVALANCHE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_AVALANCHE_BASEPOWERCALLBACK];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// AttackRecord.flags bit 2: some record from this source did positive damage this turn
/// (state/mod.rs record_attack sets it for `damage > 0`; prune_attacks clears it at end of turn).
const RECORD_DAMAGED_THIS_TURN: u8 = 4;
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_AVALANCHE_BASEPOWERCALLBACK => base_power_callback(b, cx),
        _ => panic!("unexpected Avalanche function site"),
    }
}

// data/moves.ts:912-921 basePowerCallback(pokemon, target, move). Direct call, args
// [pokemon, target, move]. `pokemon.attackedBy.some(p => p.source === target && p.damage > 0 &&
// p.thisTurn)`: the per-source AttackRecord of `pokemon` for `target` carries the
// "positive damage this turn" bit, which is exactly that predicate (a record is only kept while
// its source is active, mirroring battle.ts:1706-1713). PRNG: none.
fn base_power_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let target = mon_arg(b, cx, 1);
    let base_power = support::live_base_power(b, cx, 2);
    let record = &b.state.pokemon[pokemon.0 as usize].attacks[target.0 as usize];
    if record.flags & RECORD_DAMAGED_THIS_TURN != 0 {
        return Relay::Number(base_power * 2.0);
    }
    Relay::Number(base_power)
}
