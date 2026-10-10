//! Ports data/moves.ts:12787-12823 (`noretreat`): onTry only (the embedded condition is
//! `conditions/noretreat.rs`).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none.
use crate::{
    Battle,
    actions::MoveHandle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_NORETREAT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_NORETREAT_ONTRY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:12797-12802 onTry(source, target, move): fails (false) if the user already has the
        // `noretreat` volatile; if it is already `trapped`, `delete move.volatileStatus` so the move still
        // boosts but no new volatile is added. Returns false or undefined. PRNG: none.
        dex::HOOK_MOVE_NORETREAT_ONTRY => {
            let source = mon_arg(b, cx, 0);
            let mv = move_arg(b, cx, 2);
            if b.get_volatile(source, dex::CONDITION_NORETREAT).is_some() {
                return Relay::FAIL;
            }
            if b.get_volatile(source, dex::CONDITION_TRAPPED).is_some() {
                b.active_move_mut(MoveHandle(mv)).effects.volatile_status = EffectId::NONE;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected No Retreat function site"),
    }
}
