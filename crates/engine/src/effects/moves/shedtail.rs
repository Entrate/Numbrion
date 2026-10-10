//! Faithful port of pinned Showdown 7332b60. Source/draw notes at each handler.
#![allow(unused_imports)]
use crate::effects::registry::moves_batonpass::support::{self, effect, handle, volatile};
use crate::{
    Battle,
    actions::*,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg as mon, optional_id},
    },
    event::{EffectRef, EventArg, HookCtx, Relay, RunEventOptions},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::{
        mon_flags,
        scratch::{OrderedBoosts, move_runtime},
    },
};
pub const ID: EffectId = dex::MOVE_SHEDTAIL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_SHEDTAIL_ONTRYHIT,
    dex::HOOK_MOVE_SHEDTAIL_ONHIT,
    dex::HOOK_MOVE_SHEDTAIL_SELF_ONHIT,
];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:16171-16186. PRNG: none.
        dex::HOOK_MOVE_SHEDTAIL_ONTRYHIT => {
            let t = mon(b, cx, 0);
            let p = b.state.pokemon[t.0 as usize];
            if b.can_switch(t.side()) == 0
                || volatile(b, t, const { optional_id(dex::CONDITIONS_DATA, "commanded") })
            {
                b.add(LogEntry::new("-fail", &[LogArg::Mon(t)], &[]));
                return Relay::NotFail;
            }
            if b.get_volatile(t, dex::CONDITION_SUBSTITUTE).is_some() {
                b.add(LogEntry::new(
                    "-fail",
                    &[LogArg::Mon(t), LogArg::Text("move: Shed Tail")],
                    &[],
                ));
                return Relay::NotFail;
            }
            if f64::from(p.hp) <= (f64::from(p.max_hp) / 2.).ceil() {
                b.add(LogEntry::new(
                    "-fail",
                    &[LogArg::Mon(t), LogArg::Text("move: Shed Tail")],
                    &[LogTag::Bare("weak")],
                ));
                return Relay::NotFail;
            }
            Relay::Undefined
        }
        // data/moves.ts:16187-16189. PRNG: nested damage events only.
        dex::HOOK_MOVE_SHEDTAIL_ONHIT => {
            let t = mon(b, cx, 0);
            let hp = f64::from(b.state.pokemon[t.0 as usize].max_hp);
            b.direct_damage((hp / 2.).ceil(), None, Attribution::DEFAULT);
            Relay::Undefined
        }
        // data/moves.ts:16191-16193. PRNG: none.
        dex::HOOK_MOVE_SHEDTAIL_SELF_ONHIT => {
            let s = mon(b, cx, 0);
            b.state.pokemon[s.0 as usize].flags |= mon_flags::SKIP_BEFORE_SWITCH_OUT;
            Relay::Undefined
        }
        _ => panic!("unexpected Shed Tail hook"),
    }
}
