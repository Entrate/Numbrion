//! data/moves.ts:18224-18227. Mutable type is ???, TypeId20. No draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::MOVE_STRUGGLE;
pub const HOOKS: &[HookId] = &[HookId(158)];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const ONMODIFYMOVE: HookId = HookId(158);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, ONMODIFYMOVE);
    let mov = move_arg(b, cx, 0);
    let m = mon_arg(b, cx, 1);
    b.active_move_mut(crate::actions::MoveHandle(mov)).move_type = TypeId(20);
    b.add(LogEntry::new(
        "-activate",
        &[LogArg::Mon(m), LogArg::Text("move: Struggle")],
        &[],
    ));
    Relay::Undefined
}
