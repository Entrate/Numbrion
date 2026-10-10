//! Ports data/moves.ts:17945-17956 (embedded side condition of Sticky Web).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none; boost events may draw.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_STICKYWEB;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_STICKYWEB_ONSIDESTART,
    dex::HOOK_CONDITION_STICKYWEB_ONSWITCHIN,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:17946 onSideStart: `-sidestart|side|move: Sticky Web`. PRNG: none.
        dex::HOOK_CONDITION_STICKYWEB_ONSIDESTART => {
            support::side_line(b, cx, "-sidestart", "move: Sticky Web")
        }
        // data/moves.ts:17949 onSwitchIn(pokemon): grounded without Heavy-Duty Boots logs
        // `-activate|mon|move: Sticky Web` then boost({spe:-1}, mon, foe.active[0],
        // getActiveMove('stickyweb')). Returns undefined. PRNG: none directly.
        dex::HOOK_CONDITION_STICKYWEB_ONSWITCHIN => support::sticky_web_switch_in(b, cx),
        _ => panic!("unexpected Sticky Web function site"),
    }
}
