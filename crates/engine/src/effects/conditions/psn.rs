//! data/conditions.ts:135-152. Residual calls D; no direct draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::*,
    ids::*,
    log::LogSink,
    state::Status,
};
pub const ID: EffectId = dex::CONDITION_PSN;
pub const HOOKS: &[HookId] = &[HookId(752), HookId(753)];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const ONSTART: HookId = HookId(752);
const ONRESIDUAL: HookId = HookId(753);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        ONSTART => {
            log_status(b, cx, Status::Poison, "-status");
        }
        ONRESIDUAL => {
            let m = mon_arg(b, cx, 0);
            b.damage(
                b.state.pokemon[m.0 as usize].max_hp as f64 / 8.0,
                Some(m),
                crate::actions::Attribution::DEFAULT,
            );
        }
        _ => unreachable!(),
    }
    Relay::Undefined
}
