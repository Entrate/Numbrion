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
pub const ID: EffectId = dex::ABILITY_MAGICBOUNCE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_MAGICBOUNCE_ONTRYHIT,
    dex::HOOK_ABILITY_MAGICBOUNCE_ONALLYTRYHITSIDE,
];
// Payload words: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // data/abilities.ts:2440-2462. PRNG: nested useMove events, accuracy/hits only.
    let t = mon(b, cx, 0);
    let s = mon(b, cx, 1);
    let h = handle(b, cx, 2);
    let m = *b.active_move(h);
    let side = match hook {
        dex::HOOK_ABILITY_MAGICBOUNCE_ONTRYHIT => false,
        dex::HOOK_ABILITY_MAGICBOUNCE_ONALLYTRYHITSIDE => true,
        _ => panic!("unexpected Magic Bounce hook"),
    };
    if (if side { b.is_ally(t, Some(s)) } else { t == s })
        || m.runtime_flags & move_runtime::HAS_BOUNCED != 0
        || m.flags & dex::FLAG_REFLECTABLE == 0
        || b.is_semi_invulnerable(t)
    {
        return Relay::Undefined;
    }
    let user = if side {
        MonId(b.hook_state(cx).target.0)
    } else {
        t
    };
    let new = b.get_active_move(MoveInput::Dex(m.id));
    b.active_move_mut(new).runtime_flags |= move_runtime::HAS_BOUNCED;
    b.active_move_mut(new).runtime_flags &= !move_runtime::PRANKSTER_BOOSTED;
    b.use_move(
        MoveInput::Active(new),
        user,
        support::mon(s),
        EffectRef::None,
        UseMoveOptions::default(),
    );
    b.release_active_move(new);
    if side {
        b.active_move_mut(h).runtime_flags |= move_runtime::HAS_BOUNCED;
    }
    Relay::Null
}
