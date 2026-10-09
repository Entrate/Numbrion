//! Ports data/abilities.ts:3358 (onAnyAfterSetStatus); direct RNG draws are stated in react.
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
pub const ID: EffectId = dex::ABILITY_POISONPUPPETEER;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_POISONPUPPETEER_ONANYAFTERSETSTATUS];
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
// PRNG: none directly; nested core calls may draw.
fn react<H: Host>(h: &mut H, x: Hit) {
    if !x.pecharunt {
        return;
    }
    let source = x.source.unwrap();
    if source != x.owner || x.target == source || !x.is_move {
        return;
    }
    if x.status == dex::CONDITION_PSN || x.status == dex::CONDITION_TOX {
        h.volatile(x.target, dex::CONDITION_CONFUSION, None);
    }
}
#[cfg(test)]
#[path = "reactivecontact/tests.rs"]
mod tests;
