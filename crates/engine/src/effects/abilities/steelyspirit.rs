//! Ports data/abilities.ts:4590-4602 (Steely Spirit onAllyBasePower). No PRNG draws.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_STEELYSPIRIT;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_STEELYSPIRIT_ONALLYBASEPOWER];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4592-4597 onAllyBasePower(basePower, attacker, defender, move),
        // onAllyBasePowerPriority 22: a Steel-type move (the holder's own included, since the
        // source has no attacker test) gets chainModify(1.5); returns undefined. The type is read
        // from the live move, so type-changing abilities apply. Args: [basePower (relay),
        // attacker (event target), defender (event source), move].
        dex::HOOK_ABILITY_STEELYSPIRIT_ONALLYBASEPOWER => {
            let mv = move_arg(b, cx, 3);
            if move_overlay(b, mv).move_type == support::TYPE_STEEL {
                b.chain_modify(1.5, 1.0);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Steely Spirit function site"),
    }
}
