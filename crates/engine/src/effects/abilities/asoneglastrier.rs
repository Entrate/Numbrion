//! Pinned Showdown 7332b60 effect port. See handler comments for source and draws.
#![allow(unused_imports)]
use crate::effects::registry::abilities_battlebond::support::{
    boosts, effect_at, effect_has_status, effect_is_move, foe, mon_opt, type_named,
};
use crate::{
    Battle,
    actions::{
        Attribution, FormeOptions, ImmunityMessage, ImmunitySource, MoveHandle, Stat, StatOptions,
        Types,
    },
    dex::{self, Category, DataValue, HookId, MoveTarget},
    effects::{
        HookWaiver,
        support::{mon_arg as mon, move_arg, optional_id, relay_number},
    },
    event::{CallArgs, EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::{
        mon_flags,
        scratch::{MoveEffectsScratch, move_runtime},
    },
};
pub const ID: EffectId = dex::ABILITY_ASONEGLASTRIER;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_ASONEGLASTRIER_ONSTART,
    dex::HOOK_ABILITY_ASONEGLASTRIER_ONEND,
    dex::HOOK_ABILITY_ASONEGLASTRIER_ONFOETRYEATITEM,
    dex::HOOK_ABILITY_ASONEGLASTRIER_ONSOURCEAFTERFAINT,
];
// Payload: 1 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:251-261. PRNG: none. word0=unnerved; bit8=present.
        dex::HOOK_ABILITY_ASONEGLASTRIER_ONSTART => {
            if b.hook_state(cx).payload.words[0] != 0 {
                return Relay::Undefined;
            }
            let m = mon(b, cx, 0);
            b.add(LogEntry::new(
                "-ability",
                &[LogArg::Mon(m), LogArg::Text("As One")],
                &[],
            ));
            b.add(LogEntry::new(
                "-ability",
                &[LogArg::Mon(m), LogArg::Text("Unnerve")],
                &[],
            ));
            b.hook_state_mut(cx).payload.words[0] = 1;
            b.hook_state_mut(cx).present |= 1 << 8;
            Relay::Undefined
        }
        // data/abilities.ts:257. PRNG: none.
        dex::HOOK_ABILITY_ASONEGLASTRIER_ONEND => {
            b.hook_state_mut(cx).payload.words[0] = 0;
            b.hook_state_mut(cx).present |= 1 << 8;
            Relay::Undefined
        }
        // data/abilities.ts:260. PRNG: none; absent and false both allow eating.
        dex::HOOK_ABILITY_ASONEGLASTRIER_ONFOETRYEATITEM => {
            Relay::Bool(b.hook_state(cx).payload.words[0] == 0)
        }
        // data/abilities.ts:263. PRNG: nested boost events only.
        dex::HOOK_ABILITY_ASONEGLASTRIER_ONSOURCEAFTERFAINT => {
            if effect_is_move(b, effect_at(b, cx, 3)) {
                let n = relay_number(b, cx, 0) as i8;
                let m = mon(b, cx, 2);
                b.boost(
                    boosts([(Stat::Atk, n)]),
                    Some(m),
                    Attribution::from_move(m, EffectRef::Dex(dex::ABILITY_CHILLINGNEIGH)),
                    false,
                    false,
                );
            }
            Relay::Undefined
        }
        _ => panic!("unexpected asoneglastrier hook"),
    }
}

#[cfg(test)]
#[path = "identityabilities/tests.rs"]
mod tests;
