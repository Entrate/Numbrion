//! Ports data/abilities.ts:5016 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_THICKFAT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_THICKFAT_ONSOURCEMODIFYATK,
    dex::HOOK_ABILITY_THICKFAT_ONSOURCEMODIFYSPA,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5018. PRNG: no direct draws.
        dex::HOOK_ABILITY_THICKFAT_ONSOURCEMODIFYATK => support::source_modify_atk(ID, b, cx),
        // data/abilities.ts:5025. PRNG: no direct draws.
        dex::HOOK_ABILITY_THICKFAT_ONSOURCEMODIFYSPA => support::source_modify_sp_a(ID, b, cx),
        _ => panic!("unexpected thickfat hook"),
    }
}
