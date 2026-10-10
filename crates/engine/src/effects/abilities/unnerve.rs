//! Ports data/abilities.ts:5260 (Unnerve). No PRNG draws.
//!
//! # Payload map (ability state)
//!
//! `this.effectState.unnerved`: presence bit `UNNERVED_FLAG` (custom bit 8) of the holder's
//! ability state; set means `true`, clear covers both `false` and never-assigned.
//!
//! `onSwitchInPriority: 1` is a constant manifest entry (`HOOK_ABILITY_UNNERVE_ONSWITCHIN`,
//! absent value) that only orders the start event; it has no function site.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::ABILITY_UNNERVE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_UNNERVE_ONSTART,
    dex::HOOK_ABILITY_UNNERVE_ONEND,
    dex::HOOK_ABILITY_UNNERVE_ONFOETRYEATITEM,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5262 onStart(pokemon):
        //   if (this.effectState.unnerved) return;
        //   this.add('-ability', pokemon, 'Unnerve');
        //   this.effectState.unnerved = true;
        // Returns undefined. PRNG: none.
        dex::HOOK_ABILITY_UNNERVE_ONSTART => {
            if b.hook_state(cx).present & support::UNNERVED_FLAG != 0 {
                return Relay::Undefined;
            }
            let pokemon = support::mon(b, cx, 0);
            b.add(LogEntry::new(
                "-ability",
                &[LogArg::Mon(pokemon), LogArg::Text("Unnerve")],
                &[],
            ));
            b.hook_state_mut(cx).present |= support::UNNERVED_FLAG;
            Relay::Undefined
        }
        // data/abilities.ts:5267 onEnd(): `this.effectState.unnerved = false;`
        // Returns undefined. PRNG: none.
        dex::HOOK_ABILITY_UNNERVE_ONEND => {
            b.hook_state_mut(cx).present &= !support::UNNERVED_FLAG;
            Relay::Undefined
        }
        // data/abilities.ts:5270 onFoeTryEatItem(): `return !this.effectState.unnerved;`
        // A real boolean either way: `false` vetoes the foe's eatItem, `true` leaves the
        // relay true. PRNG: none.
        dex::HOOK_ABILITY_UNNERVE_ONFOETRYEATITEM => {
            Relay::Bool(b.hook_state(cx).present & support::UNNERVED_FLAG == 0)
        }
        _ => panic!("unexpected Unnerve hook"),
    }
}
