//! Ports data/moves.ts:13978-13999 (embedded volatile condition of Protect).
//! Also hosts the batch's private helper module (`protectredirection/`), shared by the other
//! `protect_redirection` files through `registry::conditions_protect::support`.
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none; nested core calls retain event draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
#[path = "protectredirection/mod.rs"]
pub(super) mod support;
use support::Shield;
pub const ID: EffectId = dex::CONDITION_PROTECT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_PROTECT_ONSTART,
    dex::HOOK_CONDITION_PROTECT_ONTRYHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:13980-13982 onStart(target): `-singleturn|target|Protect` (no "move: "
        // prefix, unlike the other three shields). Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_PROTECT_ONSTART => support::shield_start(Shield::Protect, b, cx),
        // data/moves.ts:13984-13999 onTryHit(target, source, move), onTryHitPriority 3: bypass ->
        // undefined; else smartTarget cleared or `-activate|target|move: Protect`, an Outrage
        // counter reset (raw delete of lockedmove at duration 2) and NOT_FAIL. PRNG: none directly.
        dex::HOOK_CONDITION_PROTECT_ONTRYHIT => support::shield_try_hit(Shield::Protect, b, cx),
        _ => panic!("unexpected Protect condition function site"),
    }
}
