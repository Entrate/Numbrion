//! Ports data/moves.ts:20824-20846 (embedded side condition of Wide Guard).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none; nested core calls retain event draws.
use crate::actions::MoveHandle;
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId, MoveTarget},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_WIDEGUARD;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_WIDEGUARD_ONSIDESTART,
    dex::HOOK_CONDITION_WIDEGUARD_ONTRYHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:20825-20827 onSideStart(target, source): `-singleturn|source|Wide Guard`
        // (the user, not the side; no "move: " prefix). Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_WIDEGUARD_ONSIDESTART => {
            let source = support::mon(b, cx, 1);
            support::add_text(b, "-singleturn", source, "Wide Guard");
            Relay::Undefined
        }
        // data/moves.ts:20830-20845 onTryHit(target, source, move), onTryHitPriority 4: only
        // allAdjacent / allAdjacentFoes moves are considered; then checkMoveBypassesProtect, the
        // `-activate|target|move: Wide Guard` line, the Outrage counter reset and NOT_FAIL.
        // PRNG: none directly (HitProtect may sort ties).
        dex::HOOK_CONDITION_WIDEGUARD_ONTRYHIT => {
            let target = support::mon(b, cx, 0);
            let source = support::mon(b, cx, 1);
            let mv = move_arg(b, cx, 2);
            // Wide Guard blocks all spread moves
            let kind = move_overlay(b, mv).target;
            if kind != MoveTarget::AllAdjacent && kind != MoveTarget::AllAdjacentFoes {
                return Relay::Undefined;
            }
            if b.check_move_bypasses_protect(MoveHandle(mv), source, target, true) {
                return Relay::Undefined;
            }
            support::add_text(b, "-activate", target, "move: Wide Guard");
            support::reset_lockedmove(b, source);
            Relay::NotFail
        }
        _ => panic!("unexpected Wide Guard condition function site"),
    }
}
