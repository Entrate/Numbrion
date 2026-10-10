//! Ports data/abilities.ts:4637-4661 (Storm Drain onTryHit / onAnyRedirectTarget). No direct PRNG
//! draws; the Sp. Atk boost runs the usual boost events (listeners may sort ties).
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_STORMDRAIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_STORMDRAIN_ONTRYHIT,
    dex::HOOK_ABILITY_STORMDRAIN_ONANYREDIRECTTARGET,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4638-4645 onTryHit(target, source, move): a Water move from another
        // Pokemon is absorbed: `this.boost({ spa: 1 })` (target / source / effect all default from
        // the event), `-immune|target|[from] ability: Storm Drain` when the boost returns falsy,
        // then return null.
        dex::HOOK_ABILITY_STORMDRAIN_ONTRYHIT => {
            support::absorb_try_hit(ID, support::TYPE_WATER, b, cx)
        }
        // data/abilities.ts:4646-4656 onAnyRedirectTarget(target, source, source2, move): a Water
        // move is redirected to the holder when `validTarget(holder, source, kind)` (randomNormal /
        // adjacentFoe read as normal); `-activate|holder|ability: Storm Drain` unless the holder
        // was already the target; returns the holder. Args: [current target (relay), user, user,
        // move].
        dex::HOOK_ABILITY_STORMDRAIN_ONANYREDIRECTTARGET => {
            support::absorb_redirect_target(ID, support::TYPE_WATER, b, cx)
        }
        _ => panic!("unexpected Storm Drain function site"),
    }
}
