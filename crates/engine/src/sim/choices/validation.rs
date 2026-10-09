//! OWNER C. Parse text in source check order; keep partially filled choices.
//! Ports sim/side.ts:527-1390 and sim/battle.ts:2962-3068. PRNG: none before commit.
#![allow(unused_variables)]

use super::{ChoiceError, requests::RequestUpdate};
use crate::{Battle, ids::*, log::LogSink, state::choices::SlotChoice};

/// Source move selector; numeric slots are one-based (sim/side.ts:552-629).
/// Names are normalized at the input boundary, not in the battle loop. PRNG: none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveSelection<'a> {
    Auto,
    Index(u32),
    Name(&'a str),
}

/// All parsed suffixes, including unusable mechanics whose errors are observable
/// (sim/side.ts:1215-1255,638-829). Aliases normalize to these variants. PRNG: none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChoiceModifier {
    #[default]
    None,
    Mega,
    MegaX,
    MegaY,
    ZMove,
    Ultra,
    Dynamax,
    Terastallize,
}

impl<L: LogSink> Battle<L> {
    /// Battle.choose (sim/battle.ts:2962-2981): parse, require completeness, then
    /// synchronously call lifecycle.commit_choices if every side is done.
    /// PRNG: none on rejection/noncommit; committing lifecycle draws transitively.
    pub fn choose(&mut self, side: usize, input: &str) -> Result<(), ChoiceError> {
        todo!("C: Battle.choose")
    }

    /// Side.choose (sim/side.ts:1185-1301). Check request/cantUndo before clearing;
    /// unknown tokens record an error and continue with later comma-separated
    /// tokens. Legacy suffixes remain observable error paths. PRNG: none.
    pub fn choose_side(&mut self, side: SideId, input: &str) -> Result<(), ChoiceError> {
        todo!("C: Side.choose")
    }

    /// Side.chooseMove (sim/side.ts:552-849). Preserve target checks before locked
    /// move, disabled and late modifier checks; freeze move ID and slot presence.
    /// PRNG: none; scoped priorityEvent LockMove/SemiLockMove has stable ordering.
    pub fn choose_move(
        &mut self,
        side: SideId,
        selection: MoveSelection<'_>,
        target_loc: i8,
        modifier: ChoiceModifier,
    ) -> Result<(), ChoiceError> {
        todo!("C: Side.chooseMove")
    }

    /// Side.chooseSwitch (sim/side.ts:915-1023). Parse JS parseInt or case-insensitive
    /// name/species; validate range/active/duplicate before fainted/trapped checks.
    /// Handles Revival Blessing selections. PRNG: none.
    pub fn choose_switch(
        &mut self,
        side: SideId,
        slot_text: Option<&str>,
    ) -> Result<(), ChoiceError> {
        todo!("C: Side.chooseSwitch")
    }

    /// Side.choosePass (sim/side.ts:1330-1357). False means no slot remains, not a
    /// choice error; this distinction selects Battle.choose's fallback error.
    /// PRNG: none.
    pub fn choose_pass(&mut self, side: SideId) -> Result<bool, ChoiceError> {
        todo!("C: Side.choosePass")
    }

    /// Side.chooseTeam (sim/side.ts:1031-1035). This format rejects Team Preview
    /// choices with its exact source error. PRNG: none.
    pub fn choose_team(&mut self, side: SideId, data: &str) -> Result<(), ChoiceError> {
        todo!("C: Side.chooseTeam rejection")
    }

    /// Side.chooseShift (sim/side.ts:1097-1116). Preserve index/request/data checks
    /// before the doubles-format rejection. PRNG: none.
    pub fn choose_shift(&mut self, side: SideId) -> Result<(), ChoiceError> {
        todo!("C: Side.chooseShift rejection")
    }

    /// Side.getChoiceIndex (sim/side.ts:1303-1328). Append auto-passes for fainted,
    /// commanding or non-switching slots unless explicitly choosing pass.
    /// PRNG: none; mutates the partial SideChoice.
    pub fn choice_index(&mut self, side: SideId, is_pass: bool) -> u8 {
        todo!("C: Side.getChoiceIndex")
    }

    /// Side.isChoiceDone (sim/side.ts:539-550). Waiting sides are complete; invoking
    /// getChoiceIndex may append auto-passes. PRNG: none.
    pub fn is_choice_done(&mut self, side: SideId) -> bool {
        todo!("C: Side.isChoiceDone")
    }

    /// Battle.allChoicesDone (sim/battle.ts:3058-3068). Also sets cantUndo when
    /// supportCancel is false. PRNG: none.
    pub fn all_choices_done(&mut self) -> bool {
        todo!("C: Battle.allChoicesDone")
    }

    /// Side.clearChoice (sim/side.ts:1118-1141). Compute forced switches/passes from
    /// global requestState before replacing SideChoice. PRNG: none.
    pub fn clear_choice(&mut self, side: SideId) {
        todo!("C: Side.clearChoice")
    }

    /// Side.autoChoose (sim/side.ts:1360-1384). Deterministically choose the first
    /// eligible switch/move, with the source's ten-iteration guard. PRNG: none.
    pub fn auto_choose(&mut self, side: SideId) {
        todo!("C: Side.autoChoose")
    }

    /// Battle.undoChoice (sim/battle.ts:3030-3052). Revealed request updates survive
    /// choice clearing; return the optional resent request on success. PRNG: none.
    pub fn undo_choice(&mut self, side: SideId) -> Result<Option<String>, ChoiceError> {
        todo!("C: Battle.undoChoice")
    }

    /// Side.emitChoiceError (sim/side.ts:527-537). Error tag is Unavailable only if
    /// the requested cache patch changes data. Never writes to battle log.
    /// PRNG: none; String construction is confined to boundary/error handling.
    pub fn emit_choice_error(
        &mut self,
        side: SideId,
        message: &str,
        update: Option<RequestUpdate>,
    ) -> ChoiceError {
        todo!("C: Side.emitChoiceError")
    }

    /// Structured counterpart of Side.choose/Battle.choose (side.ts:1185,
    /// battle.ts:2962). Run identical semantic checks, including joint constraints;
    /// legal_actions output is accepted. PRNG: none until a successful commit.
    pub fn choose_typed(&mut self, side: SideId, slots: &[SlotChoice]) -> Result<(), ChoiceError> {
        todo!("C: typed choices")
    }

    /// Side.getChoice (sim/side.ts:323-351). Produce the normalized input-log choice
    /// only at a boundary; targets include their positive '+' prefix. PRNG: none.
    pub fn choice_text(&self, side: SideId) -> String {
        todo!("C: Side.getChoice")
    }
}
