//! Ports data/abilities.ts:5237 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_UNBURDEN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_UNBURDEN_ONAFTERUSEITEM,
    dex::HOOK_ABILITY_UNBURDEN_ONTAKEITEM,
    dex::HOOK_ABILITY_UNBURDEN_ONEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5238. PRNG: no direct draws.
        dex::HOOK_ABILITY_UNBURDEN_ONAFTERUSEITEM => support::after_use_item(ID, b, cx),
        // data/abilities.ts:5242. PRNG: no direct draws.
        dex::HOOK_ABILITY_UNBURDEN_ONTAKEITEM => support::take_item(ID, b, cx),
        // data/abilities.ts:5245. PRNG: no direct draws.
        dex::HOOK_ABILITY_UNBURDEN_ONEND => support::end(ID, b, cx),
        _ => panic!("unexpected unburden hook"),
    }
}
