//! Ports data/moves.ts:17620-17636 (`spiritshackle`): the 100% secondary's onHit.
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none (the secondary roll is the move executor's).
//! `secondary` and `secondaries.0` are the same function in the generated manifest; both sites share one body.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::mon_flags,
};
pub const ID: EffectId = dex::MOVE_SPIRITSHACKLE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_SPIRITSHACKLE_SECONDARY_ONHIT,
    dex::HOOK_MOVE_SPIRITSHACKLE_SECONDARIES_0_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_SPIRITSHACKLE_SECONDARY_ONHIT
        | dex::HOOK_MOVE_SPIRITSHACKLE_SECONDARIES_0_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Spirit Shackle function site"),
    }
}
// data/moves.ts:17629-17631 secondary.onHit(target, source, move):
// `if (source.isActive) target.addVolatile('trapped', source, move, 'trapper');`
// The `trapped` volatile (source = user, sourceEffect = the move) is linked to the user's `trapper`
// volatile, so it ends when the user leaves. Returns undefined. PRNG: none directly.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let mv = move_arg(b, cx, 2);
    if b.state.pokemon[source.0 as usize].flags & mon_flags::ACTIVE != 0 {
        b.add_volatile(
            target,
            dex::CONDITION_TRAPPED,
            Attribution::from_move(source, EffectRef::ActiveMove(mv)),
            Some(dex::CONDITION_TRAPPER),
        );
    }
    Relay::Undefined
}
