//! Protocol output boundary. A generic NoLog instantiation removes all formatting.
use crate::ids::*;
use crate::{
    state::{BattleState, scratch::EffectRef},
    teams::TeamDefs,
};
#[derive(Clone, Copy, Debug)]
pub enum LogArg<'a> {
    Text(&'a str),
    Mon(MonId),
    Effect(EffectRef),
    Type(TypeId),
    Number(i32),
    Health(MonId),
    Details(MonId),
}
#[derive(Clone, Copy, Debug)]
pub struct LogEntry<'a> {
    pub command: &'static str,
    pub args: &'a [LogArg<'a>],
    pub split_side: Option<SideId>,
}
#[derive(Clone, Copy, Debug)]
pub enum MoveLineEdit {
    Still,
    Miss,
    NoTarget,
    Spread(u8),
}
pub trait LogSink {
    const ENABLED: bool;
    fn emit(&mut self, state: &BattleState, teams: &TeamDefs, entry: LogEntry<'_>);
    fn edit_move(&mut self, edit: MoveLineEdit);
}
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLog;
impl LogSink for NoLog {
    const ENABLED: bool = false;
    fn emit(&mut self, _: &BattleState, _: &TeamDefs, _: LogEntry<'_>) {}
    fn edit_move(&mut self, _: MoveLineEdit) {}
}
