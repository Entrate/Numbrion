//! OWNER C. Fixed-size accepted-action options for training, without formatting.
//! Derived from side.ts:552-849,915-1023,1330-1357 and battle.ts:2403-2438.
//! This is authoritative legality: hidden information may remove an option that
//! the public cached request still advertises. Enumeration never reveals/patches
//! requests, executes hooks, emits logs or advances the PRNG.
#![allow(unused_variables)]

use crate::{
    Battle,
    dex::MoveTarget,
    ids::*,
    log::LogSink,
    state::choices::{ChosenMoveKind, SlotChoice},
};

/// Choices of signed Showdown targetLoc (battle.ts:2403-2438). Bit order is
/// -2,-1,0,+1,+2; zero means automatic targeting, not a missing choice. PRNG: none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TargetMask(pub u8);

/// One legal move selection (side.ts:552-849), including locked/Recharge/Struggle
/// forms. `move_slot=255` means absent; target_type is the post-restriction target.
/// PRNG: none during enumeration; automatic target selection draws at commit.
#[derive(Clone, Copy, Debug, Default)]
pub struct LegalMove {
    pub move_id: EffectId,
    pub move_kind: ChosenMoveKind,
    pub move_slot: u8,
    pub target_type: MoveTarget,
    pub targets: TargetMask,
    pub can_terastallize: bool,
}

/// Structured choices for one active position (side.ts:552,915,1330). Switch and
/// revival masks use current party indices 0..5. `automatic_pass` is inserted by
/// getChoiceIndex; it remains explicit in structured joint choices. PRNG: none.
#[derive(Clone, Copy, Debug, Default)]
pub struct LegalSlot {
    pub pokemon: MonId,
    pub moves: [LegalMove; 4],
    pub move_count: u8,
    pub switch_mask: u8,
    pub revival_mask: u8,
    pub can_pass: bool,
    pub automatic_pass: bool,
}

/// Constraints checked jointly across two slots (side.ts:978,824,1337-1341).
/// Incoming Pokémon must be distinct and at most one move may request Tera.
/// forced switch/pass counts describe the source's replacement limits. PRNG: none.
#[derive(Clone, Copy, Debug, Default)]
pub struct JointConstraints {
    pub unique_switch_ins: bool,
    pub terastallize_budget: u8,
    pub forced_switches: u8,
    pub forced_passes: u8,
}

/// Fixed-size legal-action space (side.ts:552-849,915-1023,1330-1357).
/// A waiting side has slot_count zero; legal joint selections must satisfy the
/// per-slot options and every JointConstraints member. PRNG: none.
#[derive(Clone, Copy, Debug, Default)]
pub struct LegalActions {
    pub side: SideId,
    pub slots: [LegalSlot; 2],
    pub slot_count: u8,
    pub constraints: JointConstraints,
}

impl<L: LogSink> Battle<L> {
    /// Authoritative options consistent with choose_move/switch/pass acceptance
    /// (side.ts:552,915,1330). Read the frozen request plus actual disabling and
    /// trapping state; never run mutating hooks or expose request updates.
    /// PRNG: none; consumes no draws for automatically targeted moves.
    pub fn legal_actions(&self, side: SideId) -> LegalActions {
        todo!("C: authoritative legal actions")
    }

    /// Shared acceptance check for typed joint choices and the text validator
    /// (side.ts:552-849,915-1023,1330-1357). No partial choice mutation; hidden
    /// disclosure errors are handled by choose_typed/choose_side. PRNG: none.
    pub fn is_legal_joint_choice(&self, side: SideId, slots: &[SlotChoice]) -> bool {
        todo!("C: joint legal-choice validation")
    }
}
