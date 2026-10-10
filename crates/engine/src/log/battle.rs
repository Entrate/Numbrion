//! OWNER T: protocol-facing Battle methods. Counting is required even with NoLog.
use super::*;
use crate::Battle;

/// The one `once` hint in scope: Illusion Level Mod (data/abilities.ts:2085). Showdown keeps a
/// `hints` set keyed by the text (or `pN|text`); here its single member is one state bit.
const ONCE_HINT_UNSUPPORTED: &str =
    "once-hint with a side is not reachable in the scoped format (only Illusion Level Mod uses `once`)";

impl<L: LogSink> Battle<L> {
    /// Ports battle.ts:3091-3113. PRNG: none; count without formatting for NoLog.
    pub fn add(&mut self, entry: LogEntry<'_>) {
        self.scratch.unsent_lines = self
            .scratch
            .unsent_lines
            .checked_add(entry.logical_lines())
            .expect("logical line count overflow");
        if L::ENABLED {
            let view = LogView {
                state: &self.state,
                teams: &self.teams,
                player_names: &self.names,
                moves: &self.scratch.moves,
            };
            self.log.emit(view, entry);
        }
    }

    /// Ports battle.ts:3115-3120. PRNG: none. Records which line attrLastMove may edit.
    ///
    /// Counts one logical line like `add`. The sink repoints `lastMoveLine` at the entry it
    /// appends (TextLog does so for the `move` and `-anim` commands, the only `addMove` callers),
    /// so a nested move that adds its own line steals the pointer from the outer one.
    pub fn add_move(&mut self, entry: LogEntry<'_>) {
        self.add(entry);
    }

    /// Ports battle.ts:3121-3144. PRNG: none. Does not append/count new lines.
    pub fn attr_last_move(&mut self, edit: MoveLineEdit<'_>) {
        if L::ENABLED {
            let view = LogView {
                state: &self.state,
                teams: &self.teams,
                player_names: &self.names,
                moves: &self.scratch.moves,
            };
            self.log.edit_move(view, edit);
        }
    }

    /// Ports battle.ts:3069-3079. PRNG: none. Honor once/per-side hint keys.
    ///
    /// `hint(text, once, side)`: skipped when its key is already in `hints`; a `side` turns the
    /// message into a secret-only split (`|split|pN`, `|-hint|text`, `""`); only `once` hints
    /// are remembered. The only `once` hint in scope is Illusion Level Mod's, tracked by
    /// `BattleState::illusion_hint`.
    pub fn hint(&mut self, text: LogArg<'_>, once: bool, side: Option<SideId>) {
        if once {
            assert!(side.is_none(), "{ONCE_HINT_UNSUPPORTED}");
            if self.state.illusion_hint {
                return;
            }
        }
        let args = [text];
        match side {
            Some(side) => self.add(LogEntry::split("-hint", &args, &[], side, true)),
            None => self.add(LogEntry::new("-hint", &args, &[])),
        }
        if once {
            self.state.illusion_hint = true;
        }
    }

    /// Ports battle.ts:191-316,3253. PRNG: none. Called once by start; realizes four
    /// buffered constructor lines, resets count before emitting to avoid double counting.
    ///
    /// The four lines the constructor and the two `setPlayer` calls wrote before the battle
    /// started, in order: `|t:|` (the timestamp is normalized away), `|gametype|doubles`,
    /// `|player|p1|<name>||`, `|player|p2|<name>||` (empty avatar and rating). `|gen|9`,
    /// `|tier|...`, the `|rule|` lines (rule Begin hooks), `|teamsize|`, `|start|` and the turn-loop
    /// `|` / `|t:|` pair are added later through `add` by `start`/`run_start_action`.
    pub fn emit_opening_log(&mut self) {
        // Pre-start the four lines were only counted; `add` counts them again.
        self.scratch.unsent_lines = 0;
        self.add(LogEntry::new("t:", &[LogArg::Empty], &[]));
        self.add(LogEntry::new("gametype", &[LogArg::Text("doubles")], &[]));
        for side in [SideId(0), SideId(1)] {
            self.add(LogEntry::new(
                "player",
                &[
                    LogArg::SideId(side),
                    LogArg::PlayerName(side),
                    LogArg::Empty,
                    LogArg::Empty,
                ],
                &[],
            ));
        }
    }

    /// Ports battle.ts:3265-3278. PRNG: none. Flush logical unsent lines at input boundary.
    ///
    /// `sendUpdates` moves `sentLogPos` to `log.length`, which resets the `log.length -
    /// sentLogPos > 1000` LINE LIMIT guard of `singleEvent`. The logical count is kept for
    /// every sink, so NoLog and TextLog trip the guard at the same line.
    pub fn send_updates(&mut self) {
        self.scratch.unsent_lines = 0;
    }
}

impl Battle<TextLog> {
    /// Difftest raw log drain (battle.ts:3265-3278). PRNG: none; independent of sendUpdates.
    pub fn drain_log(&mut self, out: &mut Vec<String>) {
        self.log.drain_into(out);
    }
}
