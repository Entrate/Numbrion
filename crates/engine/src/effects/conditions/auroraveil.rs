//! Ports data/moves.ts:843-872 (embedded side condition of Aurora Veil).
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
pub const ID: EffectId = dex::CONDITION_AURORAVEIL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_AURORAVEIL_DURATIONCALLBACK,
    dex::HOOK_CONDITION_AURORAVEIL_ONANYMODIFYDAMAGE,
    dex::HOOK_CONDITION_AURORAVEIL_ONSIDESTART,
    dex::HOOK_CONDITION_AURORAVEIL_ONSIDEEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:845 durationCallback(target, source, effect): 8 with Light Clay, else 5.
        // Direct call; PRNG: none.
        dex::HOOK_CONDITION_AURORAVEIL_DURATIONCALLBACK => support::screen_duration(b, cx),
        // data/moves.ts:851 onAnyModifyDamage: returns undefined when Reflect (physical) or Light Screen
        // (special) already covers the category; otherwise non-crit, non-infiltrating hits are weakened
        // by 2732/4096 (doubles). PRNG: none.
        dex::HOOK_CONDITION_AURORAVEIL_ONANYMODIFYDAMAGE => {
            support::screen_any_modify_damage(Screen::AuroraVeil, b, cx)
        }
        // data/moves.ts:864 onSideStart: `-sidestart|side|move: Aurora Veil`. PRNG: none.
        dex::HOOK_CONDITION_AURORAVEIL_ONSIDESTART => {
            support::side_line(b, cx, "-sidestart", "move: Aurora Veil")
        }
        // data/moves.ts:869 onSideEnd: `-sideend|side|move: Aurora Veil`. PRNG: none.
        dex::HOOK_CONDITION_AURORAVEIL_ONSIDEEND => {
            support::side_line(b, cx, "-sideend", "move: Aurora Veil")
        }
        _ => panic!("unexpected Aurora Veil function site"),
    }
}
