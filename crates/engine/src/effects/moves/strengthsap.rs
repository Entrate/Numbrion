//! Ports data/moves.ts:18174 (Strength Sap). No direct PRNG draws; `boost` and `heal` run
//! their events (ChangeBoost/TryBoost/AfterEachBoost/AfterBoost, TryHeal/Heal) whose listeners
//! may speed-sort ties.
use crate::effects::registry::abilities_baddreams::support::boosts;
use crate::{
    Battle,
    actions::{Attribution, HealEffect, Stat, StatOptions},
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_STRENGTHSAP;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_STRENGTHSAP_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_STRENGTHSAP_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected strengthsap hook"),
    }
}

// data/moves.ts:18183-18188 onHit(target, source). PRNG: none directly.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    // if (target.boosts.atk === -6) return false;
    if b.state.pokemon[target.0 as usize].boosts[Stat::Atk as usize] == -6 {
        return Relay::Bool(false);
    }
    // const atk = target.getStat('atk', false, true);   (unboosted=false, unmodified=true)
    let atk = b.get_stat(
        target,
        Stat::Atk,
        StatOptions {
            unboosted: false,
            unmodified: true,
        },
    );
    // const success = this.boost({ atk: -1 }, target, source, null, false, true);
    let success = b.boost(
        boosts([(Stat::Atk, -1)]),
        Some(target),
        Attribution::from_move(source, EffectRef::None),
        false,
        true,
    );
    // return !!(this.heal(atk, source, target) || success);
    // heal runs first; `success` is only consulted when the heal result is falsy.
    let healed = b.heal(atk, Some(source), Some(target), HealEffect::Context);
    Relay::Bool(healed.truthy() || success.truthy())
}
