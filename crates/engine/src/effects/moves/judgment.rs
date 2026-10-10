//! Pinned Showdown 7332b60 effect port. See handler comments for source and draws.
#![allow(unused_imports)]
use crate::effects::registry::abilities_battlebond::support::{
    boosts, effect_at, effect_has_status, effect_is_move, foe, mon_opt, type_named,
};
use crate::{
    Battle,
    actions::{
        Attribution, FormeOptions, ImmunityMessage, ImmunitySource, MoveHandle, Stat, StatOptions,
        Types,
    },
    dex::{self, Category, DataValue, HookId, MoveTarget},
    effects::{
        HookWaiver,
        support::{mon_arg as mon, move_arg, optional_id, relay_number},
    },
    event::{CallArgs, EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::{
        mon_flags,
        scratch::{MoveEffectsScratch, move_runtime},
    },
};
pub const ID: EffectId = dex::MOVE_JUDGMENT;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_JUDGMENT_ONMODIFYTYPE];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:9829-9835. PRNG: none. Suppression applies unlike Arceus's raw item read.
        dex::HOOK_MOVE_JUDGMENT_ONMODIFYTYPE => {
            let h = MoveHandle(move_arg(b, cx, 0));
            let m = mon(b, cx, 1);
            if b.ignoring_item(m) {
                return Relay::Undefined;
            }
            let item = b.state.pokemon[m.0 as usize].item;
            if item != EffectId::NONE {
                let data = &dex::ITEMS[(item.0 - dex::ITEM_START) as usize];
                if data.on_plate != TypeId::NONE
                    && dex::effect(item).data.get(dex::FIELD_ZMOVE).is_none()
                {
                    b.active_move_mut(h).move_type = data.on_plate;
                }
            }
            Relay::Undefined
        }
        _ => panic!("unexpected judgment hook"),
    }
}
