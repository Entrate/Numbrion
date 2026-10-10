//! Ports data/abilities.ts:3413-3425 (Power Spot onAllyBasePower). No PRNG draws.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_POWERSPOT;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_POWERSPOT_ONALLYBASEPOWER];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3415-3420 onAllyBasePower(basePower, attacker, defender, move),
        // onAllyBasePowerPriority 22: an ally of the holder (not the holder itself) gets
        // chainModify([5325, 4096]); returns undefined. Args: [basePower (relay), attacker (event
        // target), defender (event source), move].
        dex::HOOK_ABILITY_POWERSPOT_ONALLYBASEPOWER => {
            let attacker = support::mon(b, cx, 1);
            if attacker != support::state_target(b, cx) {
                b.chain_modify(5325.0, 4096.0);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Power Spot function site"),
    }
}
