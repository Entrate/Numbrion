//! Ports data/moves.ts:8583-8585 (move Helping Hand onTryHit). Its volatile condition is
//! conditions/helpinghand.rs.
//! Payload: none. Direct PRNG draws: none.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::mon_flags,
};
pub const ID: EffectId = dex::MOVE_HELPINGHAND;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_HELPINGHAND_ONTRYHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:8583-8585 onTryHit(target): `if (!target.newlySwitched &&
        // !this.queue.willMove(target)) return false;` else undefined. false makes the move fail
        // with `-fail` (the ally already moved this turn). PRNG: none.
        dex::HOOK_MOVE_HELPINGHAND_ONTRYHIT => {
            let target = support::mon(b, cx, 0);
            let newly_switched =
                b.state.pokemon[target.0 as usize].flags & mon_flags::NEWLY_SWITCHED != 0;
            if !newly_switched && b.queue_will_move(target).is_none() {
                return Relay::Bool(false);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Helping Hand function site"),
    }
}
