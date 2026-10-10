//! Ports data/moves.ts:18461-18476 (Super Fang). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
#[path = "movecallbacks/mod.rs"]
mod support;
pub const ID: EffectId = dex::MOVE_SUPERFANG;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_SUPERFANG_DAMAGECALLBACK];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_SUPERFANG_DAMAGECALLBACK => damage_callback(b, cx),
        _ => panic!("unexpected Super Fang function site"),
    }
}

// data/moves.ts:18465-18467 damageCallback(pokemon, target): direct call from getDamage
// (battle-actions.ts:1595), args [pokemon, target].
// `this.clampIntRange(target.getUndynamaxedHP() / 2, 1)`; clampIntRange floors first
// (lib/utils.ts:320), so odd HP rounds down and the minimum is 1. PRNG: none.
fn damage_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    support::half_target_hp_damage(b, cx)
}
