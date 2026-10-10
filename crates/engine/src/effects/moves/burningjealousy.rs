//! Ports data/moves.ts:2071-2091 (Burning Jealousy). The only function sites are the secondary's
//! `onHit`, which Showdown exposes as both `secondary` and `secondaries[0]`.
//! No direct PRNG draws; status events may draw through their listeners.
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
pub const ID: EffectId = dex::MOVE_BURNINGJEALOUSY;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_BURNINGJEALOUSY_SECONDARY_ONHIT,
    dex::HOOK_MOVE_BURNINGJEALOUSY_SECONDARIES_0_ONHIT,
];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_BURNINGJEALOUSY_SECONDARY_ONHIT
        | dex::HOOK_MOVE_BURNINGJEALOUSY_SECONDARIES_0_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Burning Jealousy function site"),
    }
}

// data/moves.ts:2082-2086 secondary.onHit(target, source, move): Hit single event of a
// secondary, args [target, source, move]. `target?.statsRaisedThisTurn` then
// `target.trySetStatus('brn', source, move)` (pokemon.ts:1669-1672: setStatus(this.status ||
// status, ...)); the result is ignored and the handler returns undefined. PRNG: none directly.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let mv = move_arg(b, cx, 2);
    if b.state.pokemon[target.0 as usize].flags & mon_flags::STATS_RAISED != 0 {
        b.try_set_status(
            target,
            dex::CONDITION_BRN,
            Attribution::from_move(source, EffectRef::ActiveMove(mv)),
        );
    }
    Relay::Undefined
}
