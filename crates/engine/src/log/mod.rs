//! Protocol boundary: battle.ts:3069-3144; pokemon.ts:496-552; side.ts:353.
//! The interface is frozen centrally; text implementation is owned by agent T.
pub mod battle;
pub mod format;
pub mod text;
use crate::{
    ids::*,
    state::{BattleState, Status, scratch::EffectRef},
    teams::TeamDefs,
};
pub use text::TextLog;
/// Read-only formatting view, never passed to effects or retained in a snapshot.
#[derive(Clone, Copy)]
pub struct LogView<'a> {
    pub state: &'a BattleState,
    pub teams: &'a TeamDefs,
    pub player_names: &'a [String; 2],
    pub moves: &'a [Option<crate::state::scratch::ActiveMove>; 8],
}
#[derive(Clone, Copy, Debug)]
pub enum LogArg<'a> {
    Text(&'a str),
    /// Concatenate within one argument, without protocol separators or caller formatting.
    /// Ports interpolated hints/labels such as data/moves.ts:20930.
    Parts(&'a [LogArg<'a>]),
    Empty,
    Bool(bool),
    Mon(MonId),
    Side(SideId),
    SideId(SideId),
    PlayerName(SideId),
    Effect(EffectRef),
    EffectFullName(EffectRef),
    Type(TypeId),
    Status(Status),
    Number(i32),
    Decimal(f64),
    Health(MonId),
    Details(MonId),
    FullDetails(MonId),
    Species(EffectId),
    Spread {
        mons: [MonId; 4],
        len: u8,
    },
}
/// Tags retain call-site order, including duplicated [from]/[of] when TS emits them.
#[derive(Clone, Copy, Debug)]
pub enum LogTag<'a> {
    From(EffectRef),
    Of(MonId),
    Bare(&'static str),
    Value(&'static str, LogArg<'a>),
    Text(&'a str),
}
#[derive(Clone, Copy, Debug)]
pub struct LogEntry<'a> {
    pub command: &'static str,
    pub args: &'a [LogArg<'a>],
    pub tags: &'a [LogTag<'a>],
    pub split_side: Option<SideId>,
    pub secret_only: bool,
}
impl<'a> LogEntry<'a> {
    pub const fn new(
        command: &'static str,
        args: &'a [LogArg<'a>],
        tags: &'a [LogTag<'a>],
    ) -> Self {
        Self {
            command,
            args,
            tags,
            split_side: None,
            secret_only: false,
        }
    }
    pub const fn split(
        command: &'static str,
        args: &'a [LogArg<'a>],
        tags: &'a [LogTag<'a>],
        side: SideId,
        secret_only: bool,
    ) -> Self {
        Self {
            command,
            args,
            tags,
            split_side: Some(side),
            secret_only,
        }
    }
    /// Ports battle.ts:3081-3113. PRNG: none. Empty shared entries still count.
    pub const fn logical_lines(self) -> u32 {
        if self.split_side.is_some() { 3 } else { 1 }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum MoveLineEdit<'a> {
    Still,
    Miss,
    NoTarget,
    Spread(u8),
    Retarget(MonId),
    Tag(LogTag<'a>),
}
/// No strings/formatting are constructed by a disabled instantiation.
/// emit ports battle.ts:3081-3120; edit_move ports :3121-3144. PRNG: none.
pub trait LogSink {
    const ENABLED: bool;
    fn emit(&mut self, view: LogView<'_>, entry: LogEntry<'_>);
    fn edit_move(&mut self, view: LogView<'_>, edit: MoveLineEdit<'_>);
}
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLog;
impl LogSink for NoLog {
    const ENABLED: bool = false;
    fn emit(&mut self, _: LogView<'_>, _: LogEntry<'_>) {}
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {}
}
