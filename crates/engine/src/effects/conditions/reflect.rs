//! Ports data/moves.ts:14852-14877 (embedded side condition of Reflect).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none; nested core calls retain event draws.
//! Hosts the batch-private helpers in `hazardsscreens/` (sibling files use `..::conditions_reflect::support`).
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
#[path = "hazardsscreens/mod.rs"]
pub(super) mod support;
use support::Screen;
pub const ID: EffectId = dex::CONDITION_REFLECT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_REFLECT_DURATIONCALLBACK,
    dex::HOOK_CONDITION_REFLECT_ONANYMODIFYDAMAGE,
    dex::HOOK_CONDITION_REFLECT_ONSIDESTART,
    dex::HOOK_CONDITION_REFLECT_ONSIDEEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:14854 durationCallback(target, source, effect): 8 with Light Clay, else 5.
        // Direct call; PRNG: none.
        dex::HOOK_CONDITION_REFLECT_DURATIONCALLBACK => support::screen_duration(b, cx),
        // data/moves.ts:14860 onAnyModifyDamage: physical, non-crit, non-infiltrating hits on this
        // side's mons are weakened by 2732/4096 (doubles). Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_REFLECT_ONANYMODIFYDAMAGE => {
            support::screen_any_modify_damage(Screen::Reflect, b, cx)
        }
        // data/moves.ts:14869 onSideStart: `-sidestart|side|Reflect` (no "move: " prefix). PRNG: none.
        dex::HOOK_CONDITION_REFLECT_ONSIDESTART => {
            support::side_line(b, cx, "-sidestart", "Reflect")
        }
        // data/moves.ts:14874 onSideEnd: `-sideend|side|Reflect`. PRNG: none.
        dex::HOOK_CONDITION_REFLECT_ONSIDEEND => support::side_line(b, cx, "-sideend", "Reflect"),
        _ => panic!("unexpected Reflect function site"),
    }
}
