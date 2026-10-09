//! Ports data/abilities.ts:5379 (ability). No direct PRNG draws; nested core calls retain event draws.
use crate::effects::registry::abilities_aromaveil::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_WATERBUBBLE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_WATERBUBBLE_ONSOURCEMODIFYATK,
    dex::HOOK_ABILITY_WATERBUBBLE_ONSOURCEMODIFYSPA,
    dex::HOOK_ABILITY_WATERBUBBLE_ONMODIFYATK,
    dex::HOOK_ABILITY_WATERBUBBLE_ONMODIFYSPA,
    dex::HOOK_ABILITY_WATERBUBBLE_ONUPDATE,
    dex::HOOK_ABILITY_WATERBUBBLE_ONSETSTATUS,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5381. PRNG: no direct draws.
        dex::HOOK_ABILITY_WATERBUBBLE_ONSOURCEMODIFYATK => support::source_modify_atk(ID, b, cx),
        // data/abilities.ts:5387. PRNG: no direct draws.
        dex::HOOK_ABILITY_WATERBUBBLE_ONSOURCEMODIFYSPA => support::source_modify_sp_a(ID, b, cx),
        // data/abilities.ts:5392. PRNG: no direct draws.
        dex::HOOK_ABILITY_WATERBUBBLE_ONMODIFYATK => support::modify_atk(ID, b, cx),
        // data/abilities.ts:5397. PRNG: no direct draws.
        dex::HOOK_ABILITY_WATERBUBBLE_ONMODIFYSPA => support::modify_sp_a(ID, b, cx),
        // data/abilities.ts:5402. PRNG: no direct draws.
        dex::HOOK_ABILITY_WATERBUBBLE_ONUPDATE => support::update(ID, b, cx),
        // data/abilities.ts:5408. PRNG: no direct draws.
        dex::HOOK_ABILITY_WATERBUBBLE_ONSETSTATUS => support::set_status(ID, b, cx),
        _ => panic!("unexpected waterbubble hook"),
    }
}
