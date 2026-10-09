//! Faint queue flushing, result and battle termination. OWNER L.
#![allow(unused_variables)]

use crate::sim::Outcome;
use crate::{Battle, event::Relay, ids::SideId, log::LogSink, state::FaintEntry};

impl<L: LogSink> Battle<L> {
    /// Mark fainted active slots for replacement; battle.ts:2528-2537.
    /// PRNG: none. Sets fnt status and true switchFlag, not faint callbacks.
    pub fn check_fainted(&mut self) {
        todo!("L: checkFainted")
    }

    /// Flush the ordered faint queue; battle.ts:2539-2608.
    /// PRNG: BeforeFaint/Faint/AfterFaint sorting and callbacks, ability/item End.
    /// Already-ended returns Undefined; ordinary nonterminal returns false.
    pub fn faint_messages(
        &mut self,
        last_first: bool,
        force_check: bool,
        check_win: bool,
    ) -> Relay {
        todo!("L: faintMessages")
    }

    /// Resolve defeat, including simultaneous faint winner; battle.ts:2610-2621.
    /// PRNG: none. For both sides empty in Gen9, last faint target's side wins.
    pub fn check_win(&mut self, faint_data: Option<FaintEntry>) -> bool {
        todo!("L: checkWin")
    }

    /// End battle with winner or tie; battle.ts:1524-1547.
    /// PRNG: none. Emit blank line then win/tie, clear requests, preserve turn.
    pub fn win(&mut self, side: Option<SideId>) -> bool {
        todo!("L: win")
    }

    /// End in a tie; battle.ts:1520-1522. PRNG: none.
    pub fn tie(&mut self) -> bool {
        todo!("L: tie")
    }

    /// Forfeit a side in doubles; battle.ts:1549-1568. PRNG: none.
    pub fn lose(&mut self, side: SideId) -> bool {
        todo!("L: lose")
    }

    /// External timeout tiebreaker; battle.ts:1471-1518.
    /// PRNG: none; compare unfainted count, HP percentages then total HP exactly.
    pub fn tiebreak(&mut self) -> bool {
        todo!("L: tiebreak")
    }

    /// Engine-side difftest/training outcome; battle.ts:1524-1547,2610-2621.
    /// PRNG: none; None until battle ended, winner 0/1 or None for tie.
    pub fn outcome(&self) -> Option<Outcome> {
        todo!("L: outcome projection")
    }
}
