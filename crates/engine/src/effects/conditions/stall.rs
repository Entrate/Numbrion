//! Ports data/conditions.ts:437-460 (Stall: Protect / Detect / Endure style consecutive-use counter).
//! Duration 2 and `counterMax: 729` are data (the manifest supplies the duration; the cap is a
//! constant below).
//!
//! Payload map (PAYLOAD_WORDS = 1): word0 = `effectState.counter` (u32, 3, 9, 27, ... 729);
//! custom presence bit 8 (`COUNTER_PRESENT`) distinguishes an unset counter from zero.
//! Direct PRNG draws: exactly one `randomChance(1, counter)` per StallMove event
//! (conditions.ts:450), taken even when the counter is 1 (singleton draws still consume).
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::present,
};
pub const ID: EffectId = dex::CONDITION_STALL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_STALL_ONSTART,
    dex::HOOK_CONDITION_STALL_ONSTALLMOVE,
    dex::HOOK_CONDITION_STALL_ONRESTART,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// Custom presence bit 8: `effectState.counter` has been assigned.
const COUNTER_PRESENT: u32 = 1 << present::CUSTOM_START;
/// data/conditions.ts:441 `counterMax: 729` (3^6).
const COUNTER_MAX: u32 = 729;
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:442-444 onStart(): `this.effectState.counter = 3`. Returns undefined.
        dex::HOOK_CONDITION_STALL_ONSTART => {
            let state = b.hook_state_mut(cx);
            state.payload.words[0] = 3;
            state.present |= COUNTER_PRESENT;
            Relay::Undefined
        }
        // data/conditions.ts:445-453 onStallMove(pokemon): counter = effectState.counter || 1;
        // success = this.randomChance(1, counter) [one PRNG draw]; on failure the volatile is
        // deleted with a raw `delete pokemon.volatiles['stall']` (no End event); returns success.
        dex::HOOK_CONDITION_STALL_ONSTALLMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            let state = b.hook_state(cx);
            let stored = if state.present & COUNTER_PRESENT != 0 {
                state.payload.words[0]
            } else {
                0
            };
            // `this.effectState.counter || 1`: undefined and 0 both fall back to 1.
            let counter = if stored == 0 { 1 } else { stored };
            let success = b.state.prng.random_chance(1, counter);
            if !success {
                b.erase_volatile(pokemon, dex::CONDITION_STALL);
            }
            Relay::Bool(success)
        }
        // data/conditions.ts:454-459 onRestart(): if (counter < counterMax) counter *= 3;
        // effectState.duration = 2. Returns undefined (a restarted addVolatile therefore
        // reports true). An unset counter compares `undefined < 729` as false and is untouched.
        dex::HOOK_CONDITION_STALL_ONRESTART => {
            let state = b.hook_state_mut(cx);
            if state.present & COUNTER_PRESENT != 0 && state.payload.words[0] < COUNTER_MAX {
                state.payload.words[0] *= 3;
            }
            state.duration = 2;
            Relay::Undefined
        }
        _ => panic!("unexpected Stall function site"),
    }
}
