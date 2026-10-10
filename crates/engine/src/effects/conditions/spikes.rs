//! Ports data/moves.ts:17510-17526 (embedded side condition of Spikes).
//! Payload: word0 = `effectState.layers` (presence bit 8; read by Toxic Debris-style consumers the
//! same way as Toxic Spikes). Direct PRNG draws: none; the SwitchIn damage event may draw.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_SPIKES;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_SPIKES_ONSIDESTART,
    dex::HOOK_CONDITION_SPIKES_ONSIDERESTART,
    dex::HOOK_CONDITION_SPIKES_ONSWITCHIN,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:17512 onSideStart: `-sidestart|side|Spikes` (no "move: " prefix), layers = 1.
        // Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_SPIKES_ONSIDESTART => support::layered_side_start(b, cx, "Spikes"),
        // data/moves.ts:17516 onSideRestart: layers >= 3 returns false; else `-sidestart`, layers++.
        // PRNG: none.
        dex::HOOK_CONDITION_SPIKES_ONSIDERESTART => {
            support::layered_side_restart(b, cx, "Spikes", 3)
        }
        // data/moves.ts:17521 onSwitchIn(pokemon): grounded and not Heavy-Duty Boots takes
        // [0,3,4,6][layers]*maxhp/24 via this.damage (defaults: target = event target, effect =
        // Spikes). Returns undefined. PRNG: none directly.
        dex::HOOK_CONDITION_SPIKES_ONSWITCHIN => support::spikes_switch_in(b, cx),
        _ => panic!("unexpected Spikes function site"),
    }
}
