//! Ports data/abilities.ts:1534-1545 (Friend Guard onAnyModifyDamage). No PRNG draws.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_FRIENDGUARD;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_FRIENDGUARD_ONANYMODIFYDAMAGE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1535-1540 onAnyModifyDamage(damage, source, target, move): a hit on a
        // Pokemon that is not the holder but is its ally is weakened with chainModify(0.75)
        // (3072/4096), which mutates the ModifyDamage frame and returns undefined.
        // Args: [damage (relay), attacker (event target), defender (event source), move].
        dex::HOOK_ABILITY_FRIENDGUARD_ONANYMODIFYDAMAGE => {
            let defender = support::mon(b, cx, 2);
            let holder = support::state_target(b, cx);
            if defender != holder && b.is_ally(defender, Some(holder)) {
                b.chain_modify(0.75, 1.0);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Friend Guard function site"),
    }
}
