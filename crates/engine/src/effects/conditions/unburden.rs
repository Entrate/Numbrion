//! Ports data/abilities.ts:5237 (embedded condition). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::CONDITION_UNBURDEN;
pub const HOOKS: &[HookId] = &[dex::HOOK_CONDITION_UNBURDEN_ONMODIFYSPE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5249. PRNG: no direct draws.
        dex::HOOK_CONDITION_UNBURDEN_ONMODIFYSPE => support::modify_spe(ID, b, cx),
        _ => panic!("unexpected unburden hook"),
    }
}
