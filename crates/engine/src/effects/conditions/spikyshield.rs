//! Ports data/moves.ts:17549-17579 (embedded volatile condition of Spiky Shield).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none; nested core calls retain event draws.
use crate::effects::registry::conditions_protect::support::{self, Shield};
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_SPIKYSHIELD;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_SPIKYSHIELD_ONSTART,
    dex::HOOK_CONDITION_SPIKYSHIELD_ONTRYHIT,
    dex::HOOK_CONDITION_SPIKYSHIELD_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:17551-17553 onStart(target): `-singleturn|target|move: Protect`. PRNG: none.
        dex::HOOK_CONDITION_SPIKYSHIELD_ONSTART => {
            support::shield_start(Shield::SpikyShield, b, cx)
        }
        // data/moves.ts:17555-17573 onTryHit(target, source, move), onTryHitPriority 3: as Protect,
        // plus on contact (checkMoveMakesContact) `this.damage(source.baseMaxhp / 8, source,
        // target)` before returning NOT_FAIL. PRNG: none directly (damage events may sort ties).
        dex::HOOK_CONDITION_SPIKYSHIELD_ONTRYHIT => {
            support::shield_try_hit(Shield::SpikyShield, b, cx)
        }
        // data/moves.ts:17574-17578 onHit(target, source, move): guarded by move.isZOrMaxPowered,
        // which cannot be true without Z/Max moves, so it is a no-op returning undefined.
        dex::HOOK_CONDITION_SPIKYSHIELD_ONHIT => support::shield_hit_z_or_max(b, cx),
        _ => panic!("unexpected Spiky Shield condition function site"),
    }
}
