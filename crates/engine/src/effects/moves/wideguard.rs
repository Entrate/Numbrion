//! Ports data/moves.ts:20818-20823 (move Wide Guard onTry / onHitSide). Its side condition is
//! conditions/wideguard.rs; the stall counter is conditions/stall.rs.
//! Payload: none. Direct PRNG draws: none.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_WIDEGUARD;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_WIDEGUARD_ONTRY,
    dex::HOOK_MOVE_WIDEGUARD_ONHITSIDE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:20818-20820 onTry(): `return !!this.queue.willAct()`. A real boolean, so
        // a last-to-move user gets `-fail`. PRNG: none (no StallMove event, unlike Protect).
        dex::HOOK_MOVE_WIDEGUARD_ONTRY => Relay::Bool(b.queue_will_act().is_some()),
        // data/moves.ts:20821-20823 onHitSide(side, source): `source.addVolatile('stall')` (args
        // [side, source, move]). The result is dropped; returns undefined. PRNG: none directly.
        dex::HOOK_MOVE_WIDEGUARD_ONHITSIDE => {
            let source = support::mon(b, cx, 1);
            support::add_stall(b, source);
            Relay::Undefined
        }
        _ => panic!("unexpected Wide Guard function site"),
    }
}
