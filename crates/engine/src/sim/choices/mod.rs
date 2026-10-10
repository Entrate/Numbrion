//! OWNER C: text choices, cached requests and authoritative training legality.
//! Ports sim/side.ts:513-1390 and sim/battle.ts:1381-1469,2962-3068.
//! No direct PRNG draws; only a successful committing choose enters lifecycle.
//!
//! Layout:
//! * `support`    JS string/number semantics and read-only state views.
//! * `requests`   makeRequest/getRequests/getMoveRequestData and the JSON writer.
//! * `validation` the text parser and every `choose*` check, in source order.
//! * `legal`      the draw-free, mutation-free legal-action enumerator.

pub mod legal;
pub mod requests;
pub(crate) mod support;
pub mod validation;

#[cfg(test)]
mod tests;

pub use legal::{JointConstraints, LegalActions, LegalMove, LegalSlot, TargetMask};
pub use requests::{PokemonRequestData, RequestUpdate, SideRequestData};
pub use validation::{ChoiceModifier, MoveSelection};

use crate::dex::MoveTarget;

/// An error side-update, never a battle-log entry (sim/side.ts:527-537).
/// `text` is exactly what Showdown sends after `|error|`, tag included
/// (`[Invalid choice] ...` / `[Unavailable choice] ...`).
/// `resent_request_json` is present only when an update actually changed the
/// cached request; its top-level `update` key is appended last. PRNG: none.
///
/// The `testfight` client hack (side.ts:775-781) re-sends the request without any
/// error message; that single case has an empty `text`.
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
///
/// Key order of an entry is fixed: move, id, pp, maxpp, target, disabled,
/// disabledSource (the last one is appended by a hidden-information update).
pub mod move_presence {
    pub const PP: u8 = 1 << 0;
    pub const MAX_PP: u8 = 1 << 1;
    pub const TARGET: u8 = 1 << 2;
    pub const DISABLED: u8 = 1 << 3;
    pub const DISABLED_SOURCE: u8 = 1 << 4;
    /// The `target` string is "adjacentFoe" (Pollen Puff under Heal Block,
    /// pokemon.ts:1000-1004). `MoveTarget` has no AdjacentFoe variant; when this
    /// bit is set it overrides the stored `target` field. Local to choices.
    pub const ADJACENT_FOE: u8 = 1 << 5;
    pub const RECHARGE: u8 = 1 << 7;
    pub const FULL: u8 = PP | MAX_PP | TARGET | DISABLED;
    pub const LOCKED: u8 = 0;
    pub const STRUGGLE: u8 = TARGET | DISABLED;
}

/// Active-request flags (pokemon.ts:1110-1148). Absence is distinct from false;
/// neither trapped bit means key omission. Other optional booleans appear only
/// when true. PRNG: none.
///
/// Serialization order at creation: maybeDisabled, maybeLocked, trapped |
/// maybeTrapped, canTerastallize. A rejected switch (side.ts:988-998) deletes
/// `maybeTrapped` and assigns `trapped = true` on the cached object, which appends
/// the key AFTER canTerastallize; `TRAPPED_LATE` records that position.
pub mod request_flags {
    pub const MAYBE_DISABLED: u16 = 1 << 0;
    pub const MAYBE_LOCKED: u16 = 1 << 1;
    pub const MAYBE_TRAPPED: u16 = 1 << 2;
    pub const TRAPPED: u16 = 1 << 3;
    /// `trapped` was added by a hidden-information update (serialized last).
    pub const TRAPPED_LATE: u16 = 1 << 4;
    pub const CAN_TERASTALLIZE: u16 = 1 << 5;
}

/// MoveSlot.flags layout (pokemon.ts:13-23,1021-1027), shared with the disabling
/// mutators: `disabled` is true (bit 0), 'hidden' (bit 1); `used` is bit 2. True
/// disables are never overwritten by later hidden ones. PRNG: none.
pub mod slot_flags {
    use crate::actions::mutators::pokemon as m;
    pub const DISABLED_TRUE: u8 = m::SLOT_DISABLED;
    pub const DISABLED_HIDDEN: u8 = m::SLOT_HIDDEN;
    pub const DISABLED_MASK: u8 = m::SLOT_DISABLED | m::SLOT_HIDDEN;
    pub const USED: u8 = m::SLOT_USED;
}

/// Move target kinds as Showdown strings. `MoveTarget` lacks `adjacentFoe` (Max
/// moves, Pollen Puff under Heal Block) and `adjacentAllyOrSelf`, both of which are
/// reachable through the choice rules. PRNG: none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Normal,
    SelfTarget,
    Any,
    AllAdjacentFoes,
    AllySide,
    AllAdjacent,
    AdjacentAlly,
    AdjacentAllyOrSelf,
    AdjacentFoe,
    All,
    Allies,
    FoeSide,
    RandomNormal,
}

impl From<MoveTarget> for TargetKind {
    fn from(t: MoveTarget) -> Self {
        match t {
            MoveTarget::Normal => Self::Normal,
            MoveTarget::SelfTarget => Self::SelfTarget,
            MoveTarget::Any => Self::Any,
            MoveTarget::AllAdjacentFoes => Self::AllAdjacentFoes,
            MoveTarget::AllySide => Self::AllySide,
            MoveTarget::AllAdjacent => Self::AllAdjacent,
            MoveTarget::AdjacentAlly => Self::AdjacentAlly,
            MoveTarget::All => Self::All,
            MoveTarget::Allies => Self::Allies,
            MoveTarget::FoeSide => Self::FoeSide,
            MoveTarget::RandomNormal => Self::RandomNormal,
        }
    }
}

impl TargetKind {
    /// The Showdown target string (JSON `target`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::SelfTarget => "self",
            Self::Any => "any",
            Self::AllAdjacentFoes => "allAdjacentFoes",
            Self::AllySide => "allySide",
            Self::AllAdjacent => "allAdjacent",
            Self::AdjacentAlly => "adjacentAlly",
            Self::AdjacentAllyOrSelf => "adjacentAllyOrSelf",
            Self::AdjacentFoe => "adjacentFoe",
            Self::All => "all",
            Self::Allies => "allies",
            Self::FoeSide => "foeSide",
            Self::RandomNormal => "randomNormal",
        }
    }

    /// CHOOSABLE_TARGETS (battle-actions.ts:3,1544): the player supplies a location.
    pub const fn choosable(self) -> bool {
        matches!(
            self,
            Self::Normal
                | Self::Any
                | Self::AdjacentAlly
                | Self::AdjacentAllyOrSelf
                | Self::AdjacentFoe
        )
    }

    /// The closest `MoveTarget` for the typed API (AdjacentFoe and AdjacentAllyOrSelf
    /// have no exact variant; their location masks carry the exact information).
    pub const fn move_target(self) -> MoveTarget {
        match self {
            Self::Normal | Self::AdjacentFoe => MoveTarget::Normal,
            Self::SelfTarget => MoveTarget::SelfTarget,
            Self::Any => MoveTarget::Any,
            Self::AllAdjacentFoes => MoveTarget::AllAdjacentFoes,
            Self::AllySide => MoveTarget::AllySide,
            Self::AllAdjacent => MoveTarget::AllAdjacent,
            Self::AdjacentAlly | Self::AdjacentAllyOrSelf => MoveTarget::AdjacentAlly,
            Self::All => MoveTarget::All,
            Self::Allies => MoveTarget::Allies,
            Self::FoeSide => MoveTarget::FoeSide,
            Self::RandomNormal => MoveTarget::RandomNormal,
        }
    }

    /// Battle.validTargetLoc for doubles (battle.ts:2403-2438), numSlots = 2.
    /// `position` is the user's active position (0 or 1); `target_loc` is signed.
    pub const fn valid_loc(self, target_loc: i32, position: u8) -> bool {
        if target_loc == 0 {
            return true;
        }
        if target_loc > 2 || target_loc < -2 {
            return false;
        }
        let source_loc = -(position as i32 + 1);
        let is_self = source_loc == target_loc;
        let is_foe = target_loc > 0;
        let across = -(3 - target_loc);
        let is_adjacent = if target_loc > 0 {
            (across - source_loc).abs() <= 1
        } else {
            (target_loc - source_loc).abs() == 1
        };
        match self {
            Self::RandomNormal | Self::Normal => is_adjacent,
            Self::AdjacentAlly => is_adjacent && !is_foe,
            Self::AdjacentAllyOrSelf => (is_adjacent && !is_foe) || is_self,
            Self::AdjacentFoe => is_adjacent && is_foe,
            Self::Any => !is_self,
            _ => false,
        }
    }
}
