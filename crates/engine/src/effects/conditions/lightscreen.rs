//! Ports data/moves.ts:10325-10350 (embedded side condition of Light Screen).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none; nested core calls retain event draws.
use crate::effects::registry::conditions_reflect::support::{self, Screen};
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_LIGHTSCREEN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_LIGHTSCREEN_DURATIONCALLBACK,
    dex::HOOK_CONDITION_LIGHTSCREEN_ONANYMODIFYDAMAGE,
    dex::HOOK_CONDITION_LIGHTSCREEN_ONSIDESTART,
    dex::HOOK_CONDITION_LIGHTSCREEN_ONSIDEEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:10327 durationCallback(target, source, effect): 8 with Light Clay, else 5.
        // Direct call; PRNG: none.
        dex::HOOK_CONDITION_LIGHTSCREEN_DURATIONCALLBACK => support::screen_duration(b, cx),
        // data/moves.ts:10333 onAnyModifyDamage: special, non-crit, non-infiltrating hits on this
        // side's mons are weakened by 2732/4096 (doubles). Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_LIGHTSCREEN_ONANYMODIFYDAMAGE => {
            support::screen_any_modify_damage(Screen::LightScreen, b, cx)
        }
        // data/moves.ts:10342 onSideStart: `-sidestart|side|move: Light Screen`. PRNG: none.
        dex::HOOK_CONDITION_LIGHTSCREEN_ONSIDESTART => {
            support::side_line(b, cx, "-sidestart", "move: Light Screen")
        }
        // data/moves.ts:10347 onSideEnd: `-sideend|side|move: Light Screen`. PRNG: none.
        dex::HOOK_CONDITION_LIGHTSCREEN_ONSIDEEND => {
            support::side_line(b, cx, "-sideend", "move: Light Screen")
        }
        _ => panic!("unexpected Light Screen function site"),
    }
}
