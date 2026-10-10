//! Ports data/moves.ts:19389-19444 (`throatchop`): the 100% secondary's onHit (the embedded condition is
//! `conditions/throatchop.rs`).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none (the secondary roll is the move executor's).
//! `secondary` and `secondaries.0` are the same function in the generated manifest; both sites share one body.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_THROATCHOP;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_THROATCHOP_SECONDARY_ONHIT,
    dex::HOOK_MOVE_THROATCHOP_SECONDARIES_0_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_THROATCHOP_SECONDARY_ONHIT
        | dex::HOOK_MOVE_THROATCHOP_SECONDARIES_0_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Throat Chop function site"),
    }
}
// data/moves.ts:19435-19439 secondary.onHit(target): `target.addVolatile('throatchop');`
// Source and source effect default from the running Hit event (user, the move). Returns undefined.
// PRNG: none directly.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    b.add_volatile(
        target,
        dex::CONDITION_THROATCHOP,
        Attribution::DEFAULT,
        None,
    );
    Relay::Undefined
}
