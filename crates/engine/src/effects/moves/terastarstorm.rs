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
pub const ID: EffectId = dex::MOVE_TERASTARSTORM;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_TERASTARSTORM_ONMODIFYTYPE,
    dex::HOOK_MOVE_TERASTARSTORM_ONMODIFYMOVE,
];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:19249-19256. PRNG: unmodified stat query events only.
        dex::HOOK_MOVE_TERASTARSTORM_ONMODIFYTYPE => {
            let h = MoveHandle(move_arg(b, cx, 0));
            let m = mon(b, cx, 1);
            if b.state.pokemon[m.0 as usize].species == dex::SPECIES_TERAPAGOSSTELLAR {
                b.active_move_mut(h).move_type = dex::TYPE_STELLAR;
                if b.state.pokemon[m.0 as usize].terastallized != TypeId::NONE
                    && b.get_stat(
                        m,
                        Stat::Atk,
                        StatOptions {
                            unboosted: false,
                            unmodified: true,
                        },
                    ) > b.get_stat(
                        m,
                        Stat::SpA,
                        StatOptions {
                            unboosted: false,
                            unmodified: true,
                        },
                    )
                {
                    b.active_move_mut(h).category = Category::Physical;
                }
            }
            Relay::Undefined
        }
        // data/moves.ts:19257-19261. PRNG: none. Applies by current species, not tera flag.
        dex::HOOK_MOVE_TERASTARSTORM_ONMODIFYMOVE => {
            let h = MoveHandle(move_arg(b, cx, 0));
            let m = mon(b, cx, 1);
            if b.state.pokemon[m.0 as usize].species == dex::SPECIES_TERAPAGOSSTELLAR {
                b.active_move_mut(h).target = MoveTarget::AllAdjacentFoes;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected terastarstorm hook"),
    }
}
