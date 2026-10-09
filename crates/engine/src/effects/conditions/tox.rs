//! data/conditions.ts:155-187. words[0]=stage, presence bit8; no direct draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::*,
    ids::*,
    log::LogSink,
    state::Status,
};
pub const ID: EffectId = dex::CONDITION_TOX;
pub const HOOKS: &[HookId] = &[HookId(843), HookId(844), HookId(845)];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const ONSTART: HookId = HookId(843);
const ONSWITCHIN: HookId = HookId(844);
const ONRESIDUAL: HookId = HookId(845);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        ONSTART => {
            let c = b.hook_state_mut(cx);
            c.payload.words[0] = 0;
            c.present |= 1 << 8;
            log_status(b, cx, Status::Toxic, "-status");
        }
        ONSWITCHIN => {
            let c = b.hook_state_mut(cx);
            c.payload.words[0] = 0;
            c.present |= 1 << 8;
        }
        ONRESIDUAL => {
            let m = mon_arg(b, cx, 0);
            let stage = {
                let c = b.hook_state_mut(cx);
                if c.payload.words[0] < 15 {
                    c.payload.words[0] += 1;
                }
                c.payload.words[0]
            };
            let unit = (b.state.pokemon[m.0 as usize].max_hp as f64 / 16.0)
                .trunc()
                .max(1.0);
            b.damage(
                unit * stage as f64,
                Some(m),
                crate::actions::Attribution::DEFAULT,
            );
        }
        _ => unreachable!(),
    }
    Relay::Undefined
}
