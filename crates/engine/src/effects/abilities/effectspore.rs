//! Ports data/abilities.ts:1161 (onDamagingHit); direct RNG draws are stated in react.
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
pub const ID: EffectId = dex::ABILITY_EFFECTSPORE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_EFFECTSPORE_ONDAMAGINGHIT];
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
// PRNG: one random(100) after contact/immunity; nested core calls may draw.
fn react<H: Host>(h: &mut H, x: Hit) {
    let source = x.source.unwrap();
    if h.contact(source, x.target, false) && h.powder_immunity(source) {
        let r = h.random(100);
        let status = if r < 11 {
            Some(dex::CONDITION_SLP)
        } else if r < 21 {
            Some(dex::CONDITION_PAR)
        } else if r < 30 {
            Some(dex::CONDITION_PSN)
        } else {
            None
        };
        if let Some(id) = status {
            h.status(source, id, x.target, false);
        }
    }
}
#[cfg(test)]
#[path = "reactivecontact/tests.rs"]
mod tests;
