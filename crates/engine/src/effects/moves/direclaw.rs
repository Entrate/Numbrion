//! Dire Claw (moves:direclaw). Ports data/moves.ts:3629-3647.
//!
//! The 50% secondary roll is made by the core before the callback is reached; the callback
//! itself is the status lottery. Both generated function sites (`secondary.onHit` and the
//! normalised `secondaries.0.onHit`) are the same JS function and share one helper.
//! Payload: none.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};

pub const ID: EffectId = dex::MOVE_DIRECLAW;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_DIRECLAW_SECONDARY_ONHIT,
    dex::HOOK_MOVE_DIRECLAW_SECONDARIES_0_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_DIRECLAW_SECONDARY_ONHIT | dex::HOOK_MOVE_DIRECLAW_SECONDARIES_0_ONHIT => {
            on_hit(b, cx)
        }
        _ => panic!("unexpected Dire Claw function site"),
    }
}

/// data/moves.ts:3640-3643, `onHit(target, source)` of the 50% secondary.
/// PRNG: exactly one `sample` (= `random(3)`) before the status attempt, whether or not the
/// target can be statused; `trySetStatus('slp')` then draws the sleep length (random(2,5))
/// from the Sleep condition's onStart. Returns undefined.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let status = b
        .state
        .prng
        .sample(&[dex::CONDITION_PSN, dex::CONDITION_PAR, dex::CONDITION_SLP]);
    b.try_set_status(
        target,
        status,
        Attribution::from_move(source, EffectRef::None),
    );
    Relay::Undefined
}
