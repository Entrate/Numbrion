//! Effect-file authoring contract, battle.ts:641-646,845-859 and direct call sites.
//! Metadata construction makes no PRNG draws; a Hook body documents its own draws.
pub use crate::event::HookCtx;
use crate::{dex::HookId, ids::EffectId};
/// Monomorphized function bodies cannot be stored behind a generic fn pointer.
/// EffectImpl is registration metadata; build.rs generates static dispatch calls.
#[derive(Clone, Copy, Debug)]
pub struct EffectImpl {
    pub id: EffectId,
    pub hooks: &'static [HookId],
    pub payload_words: usize,
    pub waivers: &'static [HookWaiver],
}
/// Numeric identity of the exact generated function site (including nested objects).
pub type Hook = HookId;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaiverReason {
    ValidationOnly,
    OutsideFormat,
}
/// Reached waived functions still panic; this is a coverage annotation, never a no-op.
#[derive(Clone, Copy, Debug)]
pub struct HookWaiver {
    pub hook: HookId,
    pub reason: WaiverReason,
    pub explanation: &'static str,
}
