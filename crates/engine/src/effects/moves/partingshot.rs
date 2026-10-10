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
pub const ID: EffectId = dex::MOVE_PARTINGSHOT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_PARTINGSHOT_ONHIT];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    assert_eq!(hook, HOOKS[0]);
    // data/moves.ts:13174-13179. PRNG: nested boost events only.
    let t = mon(b, cx, 0);
    let s = mon(b, cx, 1);
    let h = handle(b, cx, 2);
    let mut boosts = OrderedBoosts::default();
    boosts.values[0] = -1;
    boosts.values[2] = -1;
    boosts.order[0] = 0;
    boosts.order[1] = 2;
    boosts.present = 5;
    boosts.len = 2;
    let success = b.boost(
        boosts,
        Some(t),
        Attribution::from_move(s, EffectRef::None),
        false,
        false,
    );
    if !success.truthy() && !b.has_ability(t, &[dex::ABILITY_MIRRORARMOR]) {
        b.active_move_mut(h).effects.self_switch = dex::SelfSwitch::None;
    }
    Relay::Undefined
}
