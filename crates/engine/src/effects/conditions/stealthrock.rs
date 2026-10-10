//! Ports data/moves.ts:17824-17834 (embedded side condition of Stealth Rock).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none; the SwitchIn damage event may draw.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_STEALTHROCK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_STEALTHROCK_ONSIDESTART,
    dex::HOOK_CONDITION_STEALTHROCK_ONSWITCHIN,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:17826 onSideStart: `-sidestart|side|move: Stealth Rock`. PRNG: none.
        dex::HOOK_CONDITION_STEALTHROCK_ONSIDESTART => {
            support::side_line(b, cx, "-sidestart", "move: Stealth Rock")
        }
        // data/moves.ts:17829 onSwitchIn(pokemon): without Heavy-Duty Boots,
        // typeMod = clamp(runEffectiveness(getActiveMove('stealthrock')), -6, 6) and
        // this.damage(maxhp * 2**typeMod / 8). Returns undefined. PRNG: none directly.
        dex::HOOK_CONDITION_STEALTHROCK_ONSWITCHIN => support::stealth_rock_switch_in(b, cx),
        _ => panic!("unexpected Stealth Rock function site"),
    }
}
