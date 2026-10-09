//! Ports data/abilities.ts:3619 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_PURIFYINGSALT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_PURIFYINGSALT_ONSETSTATUS,
    dex::HOOK_ABILITY_PURIFYINGSALT_ONTRYADDVOLATILE,
    dex::HOOK_ABILITY_PURIFYINGSALT_ONSOURCEMODIFYATK,
    dex::HOOK_ABILITY_PURIFYINGSALT_ONSOURCEMODIFYSPA,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3620. PRNG: no direct draws.
        dex::HOOK_ABILITY_PURIFYINGSALT_ONSETSTATUS => support::set_status(ID, b, cx),
        // data/abilities.ts:3626. PRNG: no direct draws.
        dex::HOOK_ABILITY_PURIFYINGSALT_ONTRYADDVOLATILE => support::try_add_volatile(ID, b, cx),
        // data/abilities.ts:3633. PRNG: no direct draws.
        dex::HOOK_ABILITY_PURIFYINGSALT_ONSOURCEMODIFYATK => support::source_modify_atk(ID, b, cx),
        // data/abilities.ts:3640. PRNG: no direct draws.
        dex::HOOK_ABILITY_PURIFYINGSALT_ONSOURCEMODIFYSPA => support::source_modify_sp_a(ID, b, cx),
        _ => panic!("unexpected purifyingsalt hook"),
    }
}
