//! Ports data/abilities.ts:4312 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_SLOWSTART;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SLOWSTART_ONSTART,
    dex::HOOK_ABILITY_SLOWSTART_ONRESIDUAL,
    dex::HOOK_ABILITY_SLOWSTART_ONMODIFYATK,
    dex::HOOK_ABILITY_SLOWSTART_ONMODIFYSPE,
    dex::HOOK_ABILITY_SLOWSTART_ONEND,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4313. PRNG: no direct draws.
        dex::HOOK_ABILITY_SLOWSTART_ONSTART => support::start(ID, b, cx),
        // data/abilities.ts:4319. PRNG: no direct draws.
        dex::HOOK_ABILITY_SLOWSTART_ONRESIDUAL => support::residual(ID, b, cx),
        // data/abilities.ts:4329. PRNG: no direct draws.
        dex::HOOK_ABILITY_SLOWSTART_ONMODIFYATK => support::modify_atk(ID, b, cx),
        // data/abilities.ts:4334. PRNG: no direct draws.
        dex::HOOK_ABILITY_SLOWSTART_ONMODIFYSPE => support::modify_spe(ID, b, cx),
        // data/abilities.ts:4339. PRNG: no direct draws.
        dex::HOOK_ABILITY_SLOWSTART_ONEND => support::end(ID, b, cx),
        _ => panic!("unexpected slowstart hook"),
    }
}
