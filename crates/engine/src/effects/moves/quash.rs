//! Ports data/moves.ts:14454-14478 (Quash). PRNG: none.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::MOVE_QUASH;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_QUASH_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:14463-14470 onHit(target):
        //   if (this.activePerHalf === 1) return false;   // singles only: activePerHalf is 2 here
        //   const action = this.queue.willMove(target); if (!action) return false;
        //   action.order = 201; this.add('-activate', target, 'move: Quash');
        // `action.order = 201` mutates the queued action in place (the queue is re-sorted by the
        // turn loop); falling off the end returns undefined. PRNG: none.
        dex::HOOK_MOVE_QUASH_ONHIT => {
            let target = mon_arg(b, cx, 0);
            let Some(action) = b.queue_will_move(target) else {
                return Relay::FAIL;
            };
            b.queue_action_mut(action).order = 201;
            b.add(LogEntry::new(
                "-activate",
                &[LogArg::Mon(target), LogArg::Text("move: Quash")],
                &[],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected Quash hook"),
    }
}
