//! OWNER T: protocol-facing Battle methods. Counting is required even with NoLog.
#![allow(unused_variables)]
use super::*;
use crate::Battle;
impl<L: LogSink> Battle<L> {
    /// Ports battle.ts:3091-3113. PRNG: none; count without formatting for NoLog.
    pub fn add(&mut self, entry: LogEntry<'_>) {
        self.scratch.unsent_lines = self
            .scratch
            .unsent_lines
            .checked_add(entry.logical_lines())
            .expect("logical line count overflow");
        if L::ENABLED {
            self.log.emit(
                LogView {
                    state: &self.state,
                    teams: &self.teams,
                    player_names: &self.names,
                    moves: &self.scratch.moves,
                },
                entry,
            );
        }
    }
    /// Ports battle.ts:3115-3120. PRNG: none. Records which line attrLastMove may edit.
    pub fn add_move(&mut self, entry: LogEntry<'_>) {
        todo!("stage T: add_move")
    }
    /// Ports battle.ts:3121-3144. PRNG: none. Does not append/count new lines.
    pub fn attr_last_move(&mut self, edit: MoveLineEdit<'_>) {
        todo!("stage T: attr_last_move")
    }
    /// Ports battle.ts:3069-3079. PRNG: none. Honor once/per-side hint keys.
    pub fn hint(&mut self, text: LogArg<'_>, once: bool, side: Option<SideId>) {
        todo!("stage T: hint")
    }
    /// Ports battle.ts:191-316,3253. PRNG: none. Called once by start; realizes four
    /// buffered constructor lines, resets count before emitting to avoid double counting.
    pub fn emit_opening_log(&mut self) {
        todo!("stage T: emit_opening_log")
    }
    /// Ports battle.ts:3265-3278. PRNG: none. Flush logical unsent lines at input boundary.
    pub fn send_updates(&mut self) {
        todo!("stage T: send_updates")
    }
}
impl Battle<TextLog> {
    /// Difftest raw log drain (battle.ts:3265-3278). PRNG: none; independent of sendUpdates.
    pub fn drain_log(&mut self, out: &mut Vec<String>) {
        self.log.drain_into(out);
    }
}
