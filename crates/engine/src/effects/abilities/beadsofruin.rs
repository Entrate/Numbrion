//! Ports data/abilities.ts:384 (Beads of Ruin). No PRNG draws of its own; the SpD
//! relay chain may be reached through a ModifySpD event that speed-sorts ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_BEADSOFRUIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_BEADSOFRUIN_ONSTART,
    dex::HOOK_ABILITY_BEADSOFRUIN_ONANYMODIFYSPD,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:385-388 onStart(pokemon). PRNG: none.
        dex::HOOK_ABILITY_BEADSOFRUIN_ONSTART => support::ruin_start(ID, b, cx),
        // data/abilities.ts:389-396 onAnyModifySpD(spd, target, source, move).
        // Writes move.ruinedSpD; returns undefined after chainModify(0.75). PRNG: none.
        dex::HOOK_ABILITY_BEADSOFRUIN_ONANYMODIFYSPD => {
            support::ruin_defender(ID, support::RUIN_SPD, b, cx)
        }
        _ => panic!("unexpected beadsofruin hook"),
    }
}
