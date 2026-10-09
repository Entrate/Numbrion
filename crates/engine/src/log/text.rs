//! OWNER T: exact raw Showdown battle.log, including split triples and mutable move lines.
use super::*;
#[derive(Debug, Default)]
#[allow(dead_code)] // Used by agent T's bodies.
pub struct TextLog {
    pub entries: Vec<String>,
    pub(crate) last_move_line: Option<usize>,
    pub(crate) drain_cursor: usize,
}
impl LogSink for TextLog {
    const ENABLED: bool = true;
    /// Ports battle.ts:3081-3120. PRNG: none. Push one String per raw log entry.
    fn emit(&mut self, _view: LogView<'_>, _entry: LogEntry<'_>) {
        todo!("stage T: TextLog::emit")
    }
    /// Ports battle.ts:3121-3144. PRNG: none. Edits must preserve split-line layout.
    fn edit_move(&mut self, _view: LogView<'_>, _edit: MoveLineEdit<'_>) {
        todo!("stage T: TextLog::edit_move")
    }
}
impl TextLog {
    /// Ports battle.ts:3265-3278. PRNG: none. Append new entries; keep old lines for edits.
    pub fn drain_into(&mut self, _out: &mut Vec<String>) {
        todo!("stage T: TextLog::drain_into")
    }
}
