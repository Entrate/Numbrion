//! Ports data/abilities.ts:4821 (Sword of Ruin). No PRNG draws of its own; the Def
//! relay chain may be reached through a ModifyDef event that speed-sorts ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_SWORDOFRUIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SWORDOFRUIN_ONSTART,
    dex::HOOK_ABILITY_SWORDOFRUIN_ONANYMODIFYDEF,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4822-4825 onStart(pokemon). PRNG: none.
        dex::HOOK_ABILITY_SWORDOFRUIN_ONSTART => support::ruin_start(ID, b, cx),
        // data/abilities.ts:4826-4833 onAnyModifyDef(def, target, source, move).
        // Writes move.ruinedDef; returns undefined after chainModify(0.75). PRNG: none.
        dex::HOOK_ABILITY_SWORDOFRUIN_ONANYMODIFYDEF => {
            support::ruin_defender(ID, support::RUIN_DEF, b, cx)
        }
        _ => panic!("unexpected swordofruin hook"),
    }
}
