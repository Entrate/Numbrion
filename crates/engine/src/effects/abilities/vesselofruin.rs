//! Ports data/abilities.ts:5287 (Vessel of Ruin). No PRNG draws of its own; the SpA
//! relay chain may be reached through a ModifySpA event that speed-sorts ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_VESSELOFRUIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_VESSELOFRUIN_ONSTART,
    dex::HOOK_ABILITY_VESSELOFRUIN_ONANYMODIFYSPA,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5288-5291 onStart(pokemon). PRNG: none.
        dex::HOOK_ABILITY_VESSELOFRUIN_ONSTART => support::ruin_start(ID, b, cx),
        // data/abilities.ts:5292-5299 onAnyModifySpA(spa, source, target, move).
        // Writes move.ruinedSpA (not re-validated); returns undefined after chainModify(0.75).
        // PRNG: none.
        dex::HOOK_ABILITY_VESSELOFRUIN_ONANYMODIFYSPA => {
            support::ruin_attacker(ID, support::RUIN_SPA, b, cx)
        }
        _ => panic!("unexpected vesselofruin hook"),
    }
}
