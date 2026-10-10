//! Ports data/abilities.ts:4874 (Tablets of Ruin). No PRNG draws of its own; the Atk
//! relay chain may be reached through a ModifyAtk event that speed-sorts ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_TABLETSOFRUIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_TABLETSOFRUIN_ONSTART,
    dex::HOOK_ABILITY_TABLETSOFRUIN_ONANYMODIFYATK,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4875-4878 onStart(pokemon). PRNG: none.
        dex::HOOK_ABILITY_TABLETSOFRUIN_ONSTART => support::ruin_start(ID, b, cx),
        // data/abilities.ts:4879-4886 onAnyModifyAtk(atk, source, target, move).
        // Writes move.ruinedAtk (not re-validated); returns undefined after chainModify(0.75).
        // PRNG: none.
        dex::HOOK_ABILITY_TABLETSOFRUIN_ONANYMODIFYATK => {
            support::ruin_attacker(ID, support::RUIN_ATK, b, cx)
        }
        _ => panic!("unexpected tabletsofruin hook"),
    }
}
