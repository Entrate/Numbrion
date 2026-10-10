//! OWNER T: exact raw Showdown battle.log, including split triples and mutable move lines.
//!
//! `entries` is Showdown's `battle.log` verbatim: one `String` per raw entry, a split triple being
//! three entries (`|split|pN`, secret line, public line; the public line is `""` for a secret-only
//! message). History is never discarded: `attrLastMove` may edit any earlier line, wherever
//! `lastMoveLine` points, and `drain_into` only advances a cursor.
use super::format::{write_line, write_tag};
use super::*;

#[derive(Debug, Default)]
pub struct TextLog {
    pub entries: Vec<String>,
    /// `battle.lastMoveLine` (battle.ts:170,258): absolute index into `entries`, `None` for -1.
    /// One pointer, not a stack: a nested `useMove` steals it (battle.ts:3115-3120).
    pub(crate) last_move_line: Option<usize>,
    /// `sentLogPos` for `drain_into`; entries before it were already handed to the harness.
    pub(crate) drain_cursor: usize,
}

/// Replaces field `index` (the target of a `|move|` / `|-anim|` line) the way
/// `line.split('|')[index] = value; join('|')` does, including padding short lines with empty
/// fields (JS array holes join as empty strings).
fn set_field(line: &mut String, index: usize, value: &str) {
    let mut parts: Vec<&str> = line.split('|').collect();
    while parts.len() <= index {
        parts.push("");
    }
    parts[index] = value;
    *line = parts.join("|");
}

/// Index of the target field of `|move|POKE|Name|TARGET` (battle.ts:3133,3141).
const TARGET_FIELD: usize = 4;

impl LogSink for TextLog {
    const ENABLED: bool = true;

    fn reset(&mut self) {
        self.entries.clear();
        self.last_move_line = None;
        self.drain_cursor = 0;
    }

    /// Ports battle.ts:3081-3120. PRNG: none. Push one String per raw log entry.
    ///
    /// Plain entries are one line (the omniscient view, so `Health`/`FullDetails` never appear in
    /// them). A split entry is `|split|pN`, the secret line and either the shared line or `""`.
    /// `move` and `-anim` entries are the `addMove` callers (battle-actions.ts:457,801,898;
    /// data/moves.ts:4647,17240,17276; data/items.ts:4780; data/conditions.ts:342;
    /// data/abilities.ts:1663): they move `lastMoveLine` to the entry they append.
    fn emit(&mut self, view: LogView<'_>, entry: LogEntry<'_>) {
        match entry.split_side {
            None => {
                let mut line = String::with_capacity(64);
                write_line(&mut line, view, &entry, true);
                if matches!(entry.command, "move" | "-anim") {
                    self.last_move_line = Some(self.entries.len());
                }
                self.entries.push(line);
            }
            Some(side) => {
                self.entries.push(format!("|split|p{}", side.0 as u32 + 1));
                let mut secret = String::with_capacity(64);
                write_line(&mut secret, view, &entry, true);
                self.entries.push(secret);
                if entry.secret_only {
                    self.entries.push(String::new());
                } else {
                    let mut shared = String::with_capacity(64);
                    write_line(&mut shared, view, &entry, false);
                    self.entries.push(shared);
                }
            }
        }
    }

    /// Ports battle.ts:3121-3144. PRNG: none. Edits must preserve split-line layout.
    ///
    /// `attrLastMove(...args)`: no-op while `lastMoveLine < 0`; a `[still]` on a `|-anim|` line
    /// deletes the entry and clears the pointer; any other `[still]` first blanks the target
    /// field; then `'|' + args` is appended. `retargetLastMove` rewrites the target field.
    /// Split triples are never move lines, so their layout is untouched.
    fn edit_move(&mut self, view: LogView<'_>, edit: MoveLineEdit<'_>) {
        let Some(index) = self.last_move_line else {
            return;
        };
        if let MoveLineEdit::Retarget(mon) = edit {
            let mut ident = String::new();
            format::write_ident(&mut ident, view, mon);
            set_field(&mut self.entries[index], TARGET_FIELD, &ident);
            return;
        }
        let mut tag = String::with_capacity(32);
        match edit {
            MoveLineEdit::Still => tag.push_str("[still]"),
            MoveLineEdit::Miss => tag.push_str("[miss]"),
            MoveLineEdit::NoTarget => tag.push_str("[notarget]"),
            MoveLineEdit::Spread(packed) => write_packed_spread(&mut tag, view, packed),
            MoveLineEdit::Tag(t) => write_tag(&mut tag, view, t),
            MoveLineEdit::Retarget(_) => unreachable!(),
        }
        let still = tag == "[still]";
        if self.entries[index].starts_with("|-anim|") {
            if still {
                // Never observed in the corpus, but the source deletes the animation line.
                self.entries.remove(index);
                self.last_move_line = None;
                return;
            }
        } else if still {
            // "If no animation plays, the target should never be known."
            set_field(&mut self.entries[index], TARGET_FIELD, "");
        }
        let line = &mut self.entries[index];
        line.push('|');
        line.push_str(&tag);
    }
}

/// `MoveLineEdit::Spread(u8)` carries the hit slots in `getMoveTargets` order as: bits 0-1 the
/// count (0..=3, the most a move can hit), then each target as two bits of field-position value
/// (`side + 2 * position`: 0 `p1a`, 1 `p2a`, 2 `p1b`, 3 `p2b`). The canonical caller form is
/// `MoveLineEdit::Tag(LogTag::Value("spread", LogArg::Spread { .. }))`, which has no such limit.
fn write_packed_spread(out: &mut String, _view: LogView<'_>, packed: u8) {
    out.push_str("[spread] ");
    let count = (packed & 3) as usize;
    for i in 0..count {
        if i > 0 {
            out.push(',');
        }
        let fp = (packed >> (2 + 2 * i)) & 3;
        out.push('p');
        out.push((b'1' + (fp & 1)) as char);
        out.push((b'a' + (fp >> 1)) as char);
    }
}

impl TextLog {
    /// Ports battle.ts:3265-3278. PRNG: none. Append new entries; keep old lines for edits.
    ///
    /// Independent of `sendUpdates`: the cursor only decides what the harness has already seen.
    /// `Vec::len` can shrink below the cursor only through the (unobserved) `-anim` deletion, in
    /// which case Showdown's `log.slice(sentLogPos)` is likewise empty.
    pub fn drain_into(&mut self, out: &mut Vec<String>) {
        let start = self.drain_cursor.min(self.entries.len());
        out.extend(self.entries[start..].iter().cloned());
        self.drain_cursor = self.entries.len();
    }
}

#[cfg(test)]
mod tests;
