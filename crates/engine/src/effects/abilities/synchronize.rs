//! Ports data/abilities.ts:4859 (onAfterSetStatus); direct RNG draws are stated in react.
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
pub const ID: EffectId = dex::ABILITY_SYNCHRONIZE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SYNCHRONIZE_ONAFTERSETSTATUS];
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
    let Some(source) = x.source else {
        return;
    };
    if source == x.target
        || x.effect_id == dex::CONDITION_TOXICSPIKES
        || x.status == dex::CONDITION_SLP
        || x.status == dex::CONDITION_FRZ
    {
        return;
    }
    h.activate(x.target, ID);
    h.status(source, x.status, x.target, true);
}
#[cfg(test)]
#[path = "reactivecontact/tests.rs"]
mod tests;
