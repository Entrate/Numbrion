//! Ports data/moves.ts:14611-14627 (embedded volatile condition of Rage Powder).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none.
use crate::actions::ImmunityMessage;
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId, ImmunityId},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_RAGEPOWDER;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_RAGEPOWDER_ONSTART,
    dex::HOOK_CONDITION_RAGEPOWDER_ONFOEREDIRECTTARGET,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:14613-14615 onStart(pokemon): `-singleturn|pokemon|move: Rage Powder`.
        // Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_RAGEPOWDER_ONSTART => {
            let pokemon = support::mon(b, cx, 0);
            support::add_text(b, "-singleturn", pokemon, "move: Rage Powder");
            Relay::Undefined
        }
        // data/moves.ts:14617-14626 onFoeRedirectTarget(target, source, source2, move), priority 1
        // (+ effectOrder). Args: [current target (relay), user, user, move]. A Sky Dropped holder
        // is skipped; otherwise the user must pass `runStatusImmunity('powder')` (silent: Grass
        // types, Overcoat and friends, via the Immunity event) before `validTarget(holder, user,
        // move.target)`. On success smartTarget is cleared and the holder returned; else
        // undefined. PRNG: none (the Immunity run_event may sort listener ties).
        dex::HOOK_CONDITION_RAGEPOWDER_ONFOEREDIRECTTARGET => {
            let holder = support::state_target(b, cx);
            let user = support::mon(b, cx, 1);
            let mv = move_arg(b, cx, 3);
            if support::is_sky_dropped(b, holder) {
                return Relay::Undefined;
            }
            if b.run_status_immunity(user, ImmunityId::Powder, ImmunityMessage::Silent) {
                let kind = move_overlay(b, mv).target;
                if b.valid_target(holder, user, kind) {
                    if support::smart_target(b, mv) {
                        support::clear_smart_target(b, mv);
                    }
                    return Relay::Pokemon(holder);
                }
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Rage Powder condition function site"),
    }
}
