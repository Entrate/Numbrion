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
pub const ID: EffectId = dex::MOVE_TRICK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_TRICK_ONTRYIMMUNITY,
    dex::HOOK_MOVE_TRICK_ONHIT,
];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:19874-19876. PRNG: none.
        dex::HOOK_MOVE_TRICK_ONTRYIMMUNITY => {
            let t = mon(b, cx, 0);
            Relay::Bool(!b.has_ability(t, &[dex::ABILITY_STICKYHOLD]))
        }
        // data/moves.ts:19877-19912. PRNG: item TakeItem/AfterTakeItem/End/Start events only.
        dex::HOOK_MOVE_TRICK_ONHIT => {
            let t = mon(b, cx, 0);
            let s = mon(b, cx, 1);
            let e = effect(b, cx, 2);
            let yours = b.take_item(t, Some(s));
            let mine = b.take_item(s, None);
            let yourid = if let Relay::Effect(i) = yours {
                i
            } else {
                EffectId::NONE
            };
            let myid = if let Relay::Effect(i) = mine {
                i
            } else {
                EffectId::NONE
            };
            let fail = yours == Relay::Bool(false)
                || mine == Relay::Bool(false)
                || (!yours.truthy() && !mine.truthy());
            let fail = fail
                || (mine.truthy()
                    && !b
                        .single_event(
                            EventId::TakeItem,
                            EffectRef::Dex(myid),
                            Some(
                                b.state
                                    .effects
                                    .capture(b.state.pokemon[s.0 as usize].item_state),
                            ),
                            support::mon(t),
                            support::mon(s),
                            e,
                            mine,
                            None,
                        )
                        .truthy())
                || (yours.truthy()
                    && !b
                        .single_event(
                            EventId::TakeItem,
                            EffectRef::Dex(yourid),
                            Some(
                                b.state
                                    .effects
                                    .capture(b.state.pokemon[t.0 as usize].item_state),
                            ),
                            support::mon(s),
                            support::mon(t),
                            e,
                            yours,
                            None,
                        )
                        .truthy());
            if fail {
                if yours.truthy() {
                    b.state.pokemon[t.0 as usize].item = yourid;
                }
                if mine.truthy() {
                    b.state.pokemon[s.0 as usize].item = myid;
                }
                return Relay::Bool(false);
            }
            b.add(LogEntry::new(
                "-activate",
                &[LogArg::Mon(s), LogArg::Text("move: Trick")],
                &[LogTag::Of(t)],
            ));
            if mine.truthy() {
                b.set_item(t, myid, Attribution::DEFAULT);
                b.add(LogEntry::new(
                    "-item",
                    &[LogArg::Mon(t), LogArg::Effect(EffectRef::Dex(myid))],
                    &[LogTag::From(EffectRef::Dex(ID))],
                ));
            } else {
                b.add(LogEntry::new(
                    "-enditem",
                    &[LogArg::Mon(t), LogArg::Effect(EffectRef::Dex(yourid))],
                    &[LogTag::Bare("silent"), LogTag::From(EffectRef::Dex(ID))],
                ));
            }
            if yours.truthy() {
                b.set_item(s, yourid, Attribution::DEFAULT);
                b.add(LogEntry::new(
                    "-item",
                    &[LogArg::Mon(s), LogArg::Effect(EffectRef::Dex(yourid))],
                    &[LogTag::From(EffectRef::Dex(ID))],
                ));
            } else {
                b.add(LogEntry::new(
                    "-enditem",
                    &[LogArg::Mon(s), LogArg::Effect(EffectRef::Dex(myid))],
                    &[LogTag::Bare("silent"), LogTag::From(EffectRef::Dex(ID))],
                ));
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Trick hook"),
    }
}
