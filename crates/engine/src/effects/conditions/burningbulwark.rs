//! Ports data/moves.ts:2037-2067 (embedded volatile condition of Burning Bulwark).
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
pub const ID: EffectId = dex::CONDITION_BURNINGBULWARK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_BURNINGBULWARK_ONSTART,
    dex::HOOK_CONDITION_BURNINGBULWARK_ONTRYHIT,
    dex::HOOK_CONDITION_BURNINGBULWARK_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:2039-2041 onStart(target): `-singleturn|target|move: Protect`. PRNG: none.
        dex::HOOK_CONDITION_BURNINGBULWARK_ONSTART => {
            support::shield_start(Shield::BurningBulwark, b, cx)
        }
        // data/moves.ts:2043-2061 onTryHit(target, source, move), onTryHitPriority 3: as Protect but
        // `checkMoveBypassesProtect(move, source, target, false)` (status moves are not blocked
        // through blockStatus), plus on contact `source.trySetStatus('brn', target,
        // this.dex.getActiveMove('Burning Bulwark'))` before returning NOT_FAIL. PRNG: none directly.
        dex::HOOK_CONDITION_BURNINGBULWARK_ONTRYHIT => {
            support::shield_try_hit(Shield::BurningBulwark, b, cx)
        }
        // data/moves.ts:2062-2066 onHit(target, source, move): guarded by move.isZOrMaxPowered,
        // which cannot be true without Z/Max moves, so it is a no-op returning undefined.
        dex::HOOK_CONDITION_BURNINGBULWARK_ONHIT => support::shield_hit_z_or_max(b, cx),
        _ => panic!("unexpected Burning Bulwark condition function site"),
    }
}
