//! data/conditions.ts:223-235. No direct draws; nested Flinch dispatch may tie-shuffle.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::CONDITION_FLINCH;
pub const HOOKS: &[HookId] = &[HookId(683)];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const ONBEFOREMOVE: HookId = HookId(683);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, ONBEFOREMOVE);
    let m = mon_arg(b, cx, 0);
    b.add(LogEntry::new(
        "cant",
        &[LogArg::Mon(m), LogArg::Text("flinch")],
        &[],
    ));
    b.run_event(
        EventId::Flinch,
        EventArg::Holder(Holder::mon(m)),
        EventArg::Undefined,
        EffectRef::None,
        Relay::Undefined,
        RunEventOptions::default(),
    );
    Relay::FAIL
}
