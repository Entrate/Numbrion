//! data/conditions.ts:55-90. words[0]=startTime(bit8), words[1]=time(bit9, signed decrement). Start draws random(2,5) once; BeforeMove none.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink},
    state::Status,
};
pub const ID: EffectId = dex::CONDITION_SLP;
pub const HOOKS: &[HookId] = &[HookId(794), HookId(795)];
pub const PAYLOAD_WORDS: usize = 2;
pub const WAIVERS: &[HookWaiver] = &[];
const ONSTART: HookId = HookId(794);
const ONBEFOREMOVE: HookId = HookId(795);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        ONSTART => {
            log_status(b, cx, Status::Sleep, "-status");
            let time = b.state.prng.random_range(2, 5);
            let c = b.hook_state_mut(cx);
            c.payload.words[0] = time;
            c.payload.words[1] = time;
            c.present |= (1 << 8) | (1 << 9); /* Nightmare cannot exist in this closure. */
            Relay::Undefined
        }
        ONBEFOREMOVE => {
            let m = mon_arg(b, cx, 0);
            let _mov = move_arg(b, cx, 2);
            let early = has_ability(b, m, dex::ABILITY_EARLYBIRD);
            let c = b.hook_state_mut(cx);
            c.payload.words[1] = c.payload.words[1].wrapping_sub(1 + u32::from(early));
            if c.payload.words[1] as i32 <= 0 {
                b.cure_status(m, false);
                return Relay::Undefined;
            }
            b.add(LogEntry::new(
                "cant",
                &[LogArg::Mon(m), LogArg::Text("slp")],
                &[],
            )); /* Sleep Talk and Snore (sleepUsable) are outside this scope. */
            Relay::FAIL
        }
        _ => unreachable!(),
    }
}
