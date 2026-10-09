//! data/conditions.ts:2-23. Residual calls D; no direct PRNG draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::*,
    ids::*,
    log::LogSink,
    state::Status,
};
pub const ID: EffectId = dex::CONDITION_BRN;
pub const HOOKS: &[HookId] = &[HookId(643), HookId(644)];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const ONSTART: HookId = HookId(643);
const ONRESIDUAL: HookId = HookId(644);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        ONSTART => {
            log_status(b, cx, Status::Burn, "-status");
        }
        ONRESIDUAL => {
            let m = mon_arg(b, cx, 0);
            b.damage(
                b.state.pokemon[m.0 as usize].max_hp as f64 / 16.0,
                Some(m),
                crate::actions::Attribution::DEFAULT,
            );
        }
        _ => unreachable!(),
    }
    Relay::Undefined
}
