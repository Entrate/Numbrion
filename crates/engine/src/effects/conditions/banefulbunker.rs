//! Ports data/moves.ts:1002-1032 (embedded volatile condition of Baneful Bunker).
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
pub const ID: EffectId = dex::CONDITION_BANEFULBUNKER;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_BANEFULBUNKER_ONSTART,
    dex::HOOK_CONDITION_BANEFULBUNKER_ONTRYHIT,
    dex::HOOK_CONDITION_BANEFULBUNKER_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:1004-1006 onStart(target): `-singleturn|target|move: Protect`. PRNG: none.
        dex::HOOK_CONDITION_BANEFULBUNKER_ONSTART => {
            support::shield_start(Shield::BanefulBunker, b, cx)
        }
        // data/moves.ts:1008-1026 onTryHit(target, source, move), onTryHitPriority 3: as Protect,
        // plus on contact (checkMoveMakesContact) `source.trySetStatus('psn', target,
        // this.dex.getActiveMove('Baneful Bunker'))` before returning NOT_FAIL. PRNG: none directly.
        dex::HOOK_CONDITION_BANEFULBUNKER_ONTRYHIT => {
            support::shield_try_hit(Shield::BanefulBunker, b, cx)
        }
        // data/moves.ts:1027-1031 onHit(target, source, move): guarded by move.isZOrMaxPowered,
        // which cannot be true without Z/Max moves, so it is a no-op returning undefined.
        dex::HOOK_CONDITION_BANEFULBUNKER_ONHIT => support::shield_hit_z_or_max(b, cx),
        _ => panic!("unexpected Baneful Bunker condition function site"),
    }
}
