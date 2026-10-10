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
pub const ID: EffectId = dex::ABILITY_PICKPOCKET;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_PICKPOCKET_ONAFTERMOVESECONDARY];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/abilities.ts:3241-3258. PRNG: takeItem/setItem nested events only.
    let t = mon(b, cx, 0);
    let Some(s) = Battle::<L>::arg_mon(b.event_arg(cx, 1)) else {
        return Relay::Undefined;
    };
    let e = effect(b, cx, 2);
    let EffectRef::ActiveMove(h) = e else {
        return Relay::Undefined;
    };
    if s != t && b.active_move(MoveHandle(h)).flags & dex::FLAG_CONTACT != 0 {
        let tp = b.state.pokemon[t.0 as usize];
        let sp = b.state.pokemon[s.0 as usize];
        if tp.item != EffectId::NONE
            || tp.switch_flag != EffectId::NONE
            || tp.flags & (mon_flags::SWITCH_REQUESTED | mon_flags::FORCE_SWITCH) != 0
            || sp.flags & mon_flags::SWITCH_REQUESTED != 0
        {
            return Relay::Undefined;
        }
        let Relay::Effect(item) = b.take_item(s, Some(t)) else {
            return Relay::Undefined;
        };
        if !b.set_item(t, item, Attribution::DEFAULT) {
            b.state.pokemon[s.0 as usize].item = item;
            return Relay::Undefined;
        }
        b.add(LogEntry::new(
            "-enditem",
            &[LogArg::Mon(s), LogArg::Effect(EffectRef::Dex(item))],
            &[
                LogTag::Bare("silent"),
                LogTag::From(EffectRef::Dex(ID)),
                LogTag::Of(s),
            ],
        ));
        b.add(LogEntry::new(
            "-item",
            &[LogArg::Mon(t), LogArg::Effect(EffectRef::Dex(item))],
            &[LogTag::From(EffectRef::Dex(ID)), LogTag::Of(s)],
        ));
    }
    Relay::Undefined
}
