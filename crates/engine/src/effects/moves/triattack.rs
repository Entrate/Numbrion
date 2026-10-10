//! Tri Attack (moves:triattack). Ports data/moves.ts:19845-19864.
//!
//! The 20% secondary roll is made by the core before the callback is reached; the callback
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

pub const ID: EffectId = dex::MOVE_TRIATTACK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_TRIATTACK_SECONDARY_ONHIT,
    dex::HOOK_MOVE_TRIATTACK_SECONDARIES_0_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_TRIATTACK_SECONDARY_ONHIT | dex::HOOK_MOVE_TRIATTACK_SECONDARIES_0_ONHIT => {
            on_hit(b, cx)
        }
        _ => panic!("unexpected Tri Attack function site"),
    }
}

/// data/moves.ts:19856-19859, `onHit(target, source)` of the 20% secondary.
/// PRNG: exactly one `sample` (= `random(3)`) before the status attempt, whether or not the
/// target can be statused; a brn/par/frz attempt draws nothing further directly (nested
/// events may sort ties; Freeze/Burn/Paralysis onStart draw nothing). Returns undefined.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let status = b
        .state
        .prng
        .sample(&[dex::CONDITION_BRN, dex::CONDITION_PAR, dex::CONDITION_FRZ]);
    b.try_set_status(
        target,
        status,
        Attribution::from_move(source, EffectRef::None),
    );
    Relay::Undefined
}
