//! Ports data/moves.ts:2554-2570 (Clear Smog). No PRNG draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::MOVE_CLEARSMOG;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_CLEARSMOG_ONHIT];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_CLEARSMOG_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Clear Smog function site"),
    }
}

// data/moves.ts:2563-2566 onHit(target): Hit single event, args [target, source, move].
// `target.clearBoosts()` zeroes all seven stages, then `this.add('-clearboost', target)`.
// Returns undefined. PRNG: none.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    b.clear_boosts(target);
    b.add(LogEntry::new("-clearboost", &[LogArg::Mon(target)], &[]));
    Relay::Undefined
}
