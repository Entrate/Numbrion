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
pub const ID: EffectId = dex::ABILITY_PROTEAN;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_PROTEAN_ONPREPAREHIT];
// Payload: 1 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3499-3508. PRNG: type events only. word0=used, bit8=present.
        dex::HOOK_ABILITY_PROTEAN_ONPREPAREHIT => {
            if b.hook_state(cx).payload.words[0] != 0 {
                return Relay::Undefined;
            }
            let source = mon(b, cx, 0);
            let h = MoveHandle(move_arg(b, cx, 2));
            let mv = *b.active_move(h);
            if mv.runtime_flags & move_runtime::HAS_BOUNCED != 0 {
                return Relay::Undefined;
            }
            // Future Sight / Doom Desire (futuremove), all seven callsMove moves and Snatch are outside the scoped dex (pinned data/moves.ts).
            // There is no scoped writer of callsMove; sourceEffect cannot become Snatch.
            let ty = mv.move_type;
            if ty != TypeId::NONE && ty != type_named("???") {
                let types = b.get_types(source, false, false);
                if types.len != 1 || types.values[0] != ty {
                    if !b.set_type(
                        source,
                        Types {
                            values: [ty, TypeId::NONE, TypeId::NONE],
                            len: 1,
                        },
                        false,
                    ) {
                        return Relay::Undefined;
                    }
                    b.hook_state_mut(cx).payload.words[0] = 1;
                    b.hook_state_mut(cx).present |= 1 << 8;
                    b.add(LogEntry::new(
                        "-start",
                        &[
                            LogArg::Mon(source),
                            LogArg::Text("typechange"),
                            LogArg::Type(ty),
                        ],
                        &[LogTag::From(EffectRef::Dex(ID))],
                    ));
                }
            }
            Relay::Undefined
        }
        _ => panic!("unexpected protean hook"),
    }
}
