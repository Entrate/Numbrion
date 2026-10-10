//! Ports data/moves.ts:13972-13977 (move Protect onPrepareHit / onHit). Its volatile condition is
//! conditions/protect.rs; the stall counter is conditions/stall.rs.
//! Payload: none. Direct PRNG draws: none (the StallMove event draws in Stall's onStallMove).
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_PROTECT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_PROTECT_ONPREPAREHIT,
    dex::HOOK_MOVE_PROTECT_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:13972 onPrepareHit(pokemon): `!!this.queue.willAct() &&
        // this.runEvent('StallMove', pokemon)`. false -> the move fails with `-fail`. PRNG: the
        // StallMove event draws one randomChance(1, counter) when a stall volatile exists.
        dex::HOOK_MOVE_PROTECT_ONPREPAREHIT => support::stall_prepare_hit(b, cx),
        // data/moves.ts:13975 onHit(pokemon): `pokemon.addVolatile('stall')`. Returns undefined.
        dex::HOOK_MOVE_PROTECT_ONHIT => support::stall_hit(b, cx),
        _ => panic!("unexpected Protect function site"),
    }
}
