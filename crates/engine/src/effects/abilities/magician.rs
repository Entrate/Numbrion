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
pub const ID: EffectId = dex::ABILITY_MAGICIAN;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_MAGICIAN_ONAFTERMOVESECONDARYSELF];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/abilities.ts:2479-2496. PRNG: speedSort hit-target ties, then takeItem/setItem events.
    let s = mon(b, cx, 0);
    let e = effect(b, cx, 2);
    let EffectRef::ActiveMove(i) = e else {
        return Relay::Undefined;
    };
    let h = MoveHandle(i);
    let m = *b.active_move(h);
    let p = b.state.pokemon[s.0 as usize];
    if p.flags & mon_flags::SWITCH_REQUESTED != 0
        || p.item != EffectId::NONE
        || volatile(b, s, const { optional_id(dex::CONDITIONS_DATA, "gem") })
        || dex::key_ids!("fling").contains(m.id)
        || m.category == dex::Category::Status
    {
        return Relay::Undefined;
    }
    #[derive(Clone, Copy)]
    struct Target {
        mon: MonId,
        speed: u16,
    }
    impl crate::event::SpeedSortable for Target {
        fn sort_key(&self) -> crate::event::Priority {
            crate::event::Priority {
                speed: f64::from(self.speed),
                ..Default::default()
            }
        }
    }
    let len = m.hit_target_len as usize;
    let mut targets = [Target {
        mon: MonId::NONE,
        speed: 0,
    }; 4];
    for (i, t) in targets[..len].iter_mut().enumerate() {
        let mon = m.hit_targets[i];
        *t = Target {
            mon,
            speed: b.state.pokemon[mon.0 as usize].speed,
        };
    }
    crate::event::speed_sort(
        &mut b.state.prng,
        &mut targets[..len],
        crate::event::SortOrder::Priority,
    );
    for (i, t) in targets[..len].iter().enumerate() {
        b.active_move_mut(h).hit_targets[i] = t.mon;
    }
    for t in &targets[..len] {
        if t.mon != s {
            let Relay::Effect(item) = b.take_item(t.mon, Some(s)) else {
                continue;
            };
            if !b.set_item(s, item, Attribution::DEFAULT) {
                b.state.pokemon[t.mon.0 as usize].item = item;
                continue;
            }
            b.add(LogEntry::new(
                "-item",
                &[LogArg::Mon(s), LogArg::Effect(EffectRef::Dex(item))],
                &[LogTag::From(EffectRef::Dex(ID)), LogTag::Of(t.mon)],
            ));
            return Relay::Undefined;
        }
    }
    Relay::Undefined
}
