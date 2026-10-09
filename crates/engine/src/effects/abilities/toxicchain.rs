//! Ports data/abilities.ts:5092 (onSourceDamagingHit); direct RNG draws are stated in react.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
#[path = "reactivecontact/host.rs"]
mod host;
use host::{BattleHost, Hit, Host};
pub const ID: EffectId = dex::ABILITY_TOXICCHAIN;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_TOXICCHAIN_ONSOURCEDAMAGINGHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    let x = Hit::from_battle(b, cx, ID);
    let move_index = if x.is_move_callback {
        Some(crate::effects::support::move_arg(b, cx, 3))
    } else {
        None
    };
    let mut h = BattleHost { b, move_index };
    react(&mut h, x);
    Relay::Undefined
}
// Same source callback with statically dispatched host operations. No heap or trait objects.
// PRNG: one randomChance(3,10) only after the source guards; nested core calls may draw.
fn react<H: Host>(h: &mut H, x: Hit) {
    if h.shield_dust(x.target) || h.covert_cloak(x.target) {
        return;
    }
    if h.chance(3, 10) {
        h.status(x.target, dex::CONDITION_TOX, x.source.unwrap(), false);
    }
}
#[cfg(test)]
#[path = "reactivecontact/tests.rs"]
mod tests;
