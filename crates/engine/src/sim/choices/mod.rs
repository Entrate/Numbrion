//! OWNER C: text choices, cached requests and authoritative training legality.
//! Ports sim/side.ts:513-1390 and sim/battle.ts:1381-1469,2962-3068.
//! No direct PRNG draws; only a successful committing choose enters lifecycle.
//! All behavior is deliberately unimplemented in the Stage 2A API skeleton.

pub mod legal;
pub mod requests;
pub mod validation;

pub use legal::{JointConstraints, LegalActions, LegalMove, LegalSlot, TargetMask};
pub use requests::{PokemonRequestData, RequestUpdate, SideRequestData};
pub use validation::{ChoiceModifier, MoveSelection};

/// An error side-update, never a battle-log entry (sim/side.ts:527-537).
/// `resent_request_json` is present only when an update actually changed the
/// cached request; its top-level `update` key is appended last. PRNG: none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceError {
    pub text: String,
    pub resent_request_json: Option<String>,
}

impl core::fmt::Display for ChoiceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.text)
    }
}
impl std::error::Error for ChoiceError {}

/// Optional move-entry fields, preserving JSON omission (pokemon.ts:964-1043).
/// `move` and `id` are always present. Recharge uses id NONE and its own bit.
/// The three standard shapes are FULL, LOCKED and STRUGGLE. PRNG: none.
pub mod move_presence {
    pub const PP: u8 = 1 << 0;
    pub const MAX_PP: u8 = 1 << 1;
    pub const TARGET: u8 = 1 << 2;
    pub const DISABLED: u8 = 1 << 3;
    pub const DISABLED_SOURCE: u8 = 1 << 4;
    pub const RECHARGE: u8 = 1 << 7;
    pub const FULL: u8 = PP | MAX_PP | TARGET | DISABLED;
    pub const LOCKED: u8 = 0;
    pub const STRUGGLE: u8 = TARGET | DISABLED;
}

/// Active-request flags in source insertion order (pokemon.ts:1110-1148).
/// Absence is distinct from false; neither trapped bit means key omission.
/// Other optional booleans appear only when true. PRNG: none.
pub mod request_flags {
    pub const MAYBE_DISABLED: u16 = 1 << 0;
    pub const MAYBE_LOCKED: u16 = 1 << 1;
    pub const MAYBE_TRAPPED: u16 = 1 << 2;
    pub const TRAPPED: u16 = 1 << 3;
    pub const TRAPPED_HIDDEN: u16 = 1 << 4;
    pub const CAN_TERASTALLIZE: u16 = 1 << 5;
}
