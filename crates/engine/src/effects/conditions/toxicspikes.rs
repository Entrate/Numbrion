//! Ports data/moves.ts:19759-19789 (embedded side condition of Toxic Spikes).
//! Payload: word0 = `effectState.layers` (presence bit 8) -- the contract already consumed by
//! reactivecontact/host.rs for Toxic Debris. Direct PRNG draws: none; status events may draw.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_TOXICSPIKES;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_TOXICSPIKES_ONSIDESTART,
    dex::HOOK_CONDITION_TOXICSPIKES_ONSIDERESTART,
    dex::HOOK_CONDITION_TOXICSPIKES_ONSWITCHIN,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:19761 onSideStart: `-sidestart|side|move: Toxic Spikes`, layers = 1.
        // Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_TOXICSPIKES_ONSIDESTART => {
            support::layered_side_start(b, cx, "move: Toxic Spikes")
        }
        // data/moves.ts:19765 onSideRestart: layers >= 2 returns false; else `-sidestart`, layers++.
        // PRNG: none.
        dex::HOOK_CONDITION_TOXICSPIKES_ONSIDERESTART => {
            support::layered_side_restart(b, cx, "move: Toxic Spikes", 2)
        }
        // data/moves.ts:19770 onSwitchIn(pokemon): grounded Poison types absorb it (`-sideend ...
        // [of] mon`, removeSideCondition); Steel/Heavy-Duty Boots do nothing; otherwise
        // trySetStatus('tox' if layers >= 2 else 'psn', foe.active[0]). Returns undefined.
        // PRNG: none directly (status events).
        dex::HOOK_CONDITION_TOXICSPIKES_ONSWITCHIN => support::toxic_spikes_switch_in(b, cx),
        _ => panic!("unexpected Toxic Spikes function site"),
    }
}
