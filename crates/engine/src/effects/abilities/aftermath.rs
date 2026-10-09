//! Ports data/abilities.ts:78 (onDamagingHit); direct RNG draws are stated in react.
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
pub const ID: EffectId = dex::ABILITY_AFTERMATH;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_AFTERMATH_ONDAMAGINGHIT];
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
    if x.target_hp == 0 && h.contact(x.source.unwrap(), x.target, true) {
        h.damage(x.source.unwrap(), x.source_maxhp / 4.0, x.target);
    }
}
#[cfg(test)]
#[path = "reactivecontact/tests.rs"]
mod tests;
