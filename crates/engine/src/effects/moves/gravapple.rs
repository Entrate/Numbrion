//! Ports data/moves.ts:7731-7753 (Grav Apple). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::optional_id},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_GRAVAPPLE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_GRAVAPPLE_ONBASEPOWER];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// The Gravity pseudo-weather, absent from the generated dex when the Gravity move is outside
/// the format (NONE then never matches a live pseudo-weather).
const GRAVITY: EffectId = optional_id(dex::CONDITIONS_DATA, "gravity");
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_GRAVAPPLE_ONBASEPOWER => on_base_power(b, cx),
        _ => panic!("unexpected Grav Apple function site"),
    }
}

// data/moves.ts:7740-7744 onBasePower(basePower): runEvent('BasePower', attacker, defender,
// move, basePower, true) (battle-actions.ts:1637). `this.field.getPseudoWeather('gravity')`
// then `this.chainModify(1.5)`, which mutates the frame and returns undefined. PRNG: none.
fn on_base_power<L: LogSink>(b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    if GRAVITY != EffectId::NONE && b.get_pseudo_weather(GRAVITY).is_some() {
        b.chain_modify(1.5, 1.0);
    }
    Relay::Undefined
}
