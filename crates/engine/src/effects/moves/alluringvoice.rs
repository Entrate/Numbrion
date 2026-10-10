//! Ports data/moves.ts:282-301 (Alluring Voice). The only function sites are the secondary's
//! `onHit`, which Showdown exposes as both `secondary` and `secondaries[0]`.
//! No direct PRNG draws; confusion's Start runs `random(2, 6)` for its duration via the
//! condition mutator, not here.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::*,
    log::LogSink,
    state::mon_flags,
};
pub const ID: EffectId = dex::MOVE_ALLURINGVOICE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_ALLURINGVOICE_SECONDARY_ONHIT,
    dex::HOOK_MOVE_ALLURINGVOICE_SECONDARIES_0_ONHIT,
];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_ALLURINGVOICE_SECONDARY_ONHIT
        | dex::HOOK_MOVE_ALLURINGVOICE_SECONDARIES_0_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Alluring Voice function site"),
    }
}

// data/moves.ts:293-297 secondary.onHit(target, source, move): the Hit single event of a
// secondary (battle-actions.ts:1268) passes [target, source, move]. `target?.statsRaisedThisTurn`
// then `target.addVolatile('confusion', source, move)`; the result is ignored and the handler
// returns undefined. PRNG: none directly (Confusion's onStart draws its duration).
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let mv = move_arg(b, cx, 2);
    if b.state.pokemon[target.0 as usize].flags & mon_flags::STATS_RAISED != 0 {
        b.add_volatile(
            target,
            dex::CONDITION_CONFUSION,
            Attribution::from_move(source, EffectRef::ActiveMove(mv)),
            None,
        );
    }
    Relay::Undefined
}
