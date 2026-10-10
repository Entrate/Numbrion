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
pub const ID: EffectId = dex::MOVE_BUGBITE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_BUGBITE_ONHIT];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/moves.ts:1920-1931. PRNG: takeItem, Eat, EatItem events only.
    let t = mon(b, cx, 0);
    let s = mon(b, cx, 1);
    let h = handle(b, cx, 2);
    let item = b.state.pokemon[t.0 as usize].item;
    if b.state.pokemon[s.0 as usize].hp != 0
        && support::berry(item)
        && b.take_item(t, Some(s)).truthy()
    {
        b.add(LogEntry::new(
            "-enditem",
            &[LogArg::Mon(t), LogArg::Effect(EffectRef::Dex(item))],
            &[
                LogTag::Value("from", LogArg::Text("stealeat")),
                LogTag::Value("move", LogArg::Text("Bug Bite")),
                LogTag::Of(s),
            ],
        ));
        let c = b.state.pokemon[t.0 as usize].item_state;
        if b.single_event(
            EventId::Eat,
            EffectRef::Dex(item),
            Some(b.state.effects.capture(c)),
            support::mon(s),
            support::mon(s),
            EffectRef::ActiveMove(h.0),
            Relay::Undefined,
            None,
        )
        .truthy()
        {
            b.run_event(
                EventId::EatItem,
                support::mon(s),
                support::mon(s),
                EffectRef::ActiveMove(h.0),
                Relay::Effect(item),
                RunEventOptions::default(),
            );
            // Leppa Berry is outside this format's item closure; its staleness bookkeeping is unreachable.
        }
        if b.event_hook(EffectRef::Dex(item), EventId::Eat, dex::HookRel::On)
            .is_some()
        {
            b.state.pokemon[s.0 as usize].flags |= crate::actions::mutators::common::ATE_BERRY;
        }
    }
    Relay::Undefined
}
