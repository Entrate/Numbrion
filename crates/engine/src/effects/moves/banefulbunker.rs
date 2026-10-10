//! Ports data/moves.ts:996-1001 (move Baneful Bunker onPrepareHit / onHit). Its volatile condition
//! is conditions/banefulbunker.rs.
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
pub const ID: EffectId = dex::MOVE_BANEFULBUNKER;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_BANEFULBUNKER_ONPREPAREHIT,
    dex::HOOK_MOVE_BANEFULBUNKER_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:996 onPrepareHit(pokemon): `!!this.queue.willAct() &&
        // this.runEvent('StallMove', pokemon)`. PRNG: StallMove draws one randomChance(1, counter)
        // when a stall volatile exists.
        dex::HOOK_MOVE_BANEFULBUNKER_ONPREPAREHIT => support::stall_prepare_hit(b, cx),
        // data/moves.ts:999 onHit(pokemon): `pokemon.addVolatile('stall')`. Returns undefined.
        dex::HOOK_MOVE_BANEFULBUNKER_ONHIT => support::stall_hit(b, cx),
        _ => panic!("unexpected Baneful Bunker function site"),
    }
}
