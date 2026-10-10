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
pub const ID: EffectId = dex::MOVE_TERABLAST;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_TERABLAST_BASEPOWERCALLBACK,
    dex::HOOK_MOVE_TERABLAST_ONPREPAREHIT,
    dex::HOOK_MOVE_TERABLAST_ONMODIFYTYPE,
    dex::HOOK_MOVE_TERABLAST_ONMODIFYMOVE,
];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:19208-19213. PRNG: none. Live basePower, not immutable dex fallback.
        dex::HOOK_MOVE_TERABLAST_BASEPOWERCALLBACK => {
            let m = mon(b, cx, 0);
            let h = MoveHandle(move_arg(b, cx, 2));
            Relay::Number(
                if b.state.pokemon[m.0 as usize].terastallized == dex::TYPE_STELLAR {
                    100.
                } else {
                    b.active_move(h).base_power
                },
            )
        }
        // data/moves.ts:19219-19223. PRNG: none; uses original set teraType for animation.
        dex::HOOK_MOVE_TERABLAST_ONPREPAREHIT => {
            let source = mon(b, cx, 1);
            if b.state.pokemon[source.0 as usize].terastallized != TypeId::NONE {
                let ty =
                    b.teams.sides[source.side().0 as usize].sets[(source.0 % 6) as usize].tera_type;
                let parts = [LogArg::Text("Tera Blast "), LogArg::Type(ty)];
                b.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
                    "anim",
                    LogArg::Parts(&parts),
                )));
            }
            Relay::Undefined
        }
        // data/moves.ts:19224-19228. PRNG: none.
        dex::HOOK_MOVE_TERABLAST_ONMODIFYTYPE => {
            let h = MoveHandle(move_arg(b, cx, 0));
            let m = mon(b, cx, 1);
            if b.state.pokemon[m.0 as usize].terastallized != TypeId::NONE {
                let ty = b.teams.sides[m.side().0 as usize].sets[(m.0 % 6) as usize].tera_type;
                b.active_move_mut(h).move_type = ty;
            }
            Relay::Undefined
        }
        // data/moves.ts:19229-19236. PRNG: unmodified getStat has ModifyBoost events, never stat modifier events.
        dex::HOOK_MOVE_TERABLAST_ONMODIFYMOVE => {
            let h = MoveHandle(move_arg(b, cx, 0));
            let m = mon(b, cx, 1);
            let tera = b.state.pokemon[m.0 as usize].terastallized;
            if tera != TypeId::NONE
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
            if tera == dex::TYPE_STELLAR {
                b.active_move_mut(h).self_effect = Some(stellar_self());
            }
            Relay::Undefined
        }
        _ => panic!("unexpected terablast hook"),
    }
}

// data/moves.ts:19235. Replacement self object has only ordered {atk:-1,spa:-1}.
// PRNG: none; the core self_drops callback owns the required random(100).
fn stellar_self() -> MoveEffectsScratch {
    static EMPTY: dex::MoveEffects = dex::MoveEffects {
        boosts: &[],
        status: EffectId::NONE,
        volatile_status: EffectId::NONE,
        side_condition: EffectId::NONE,
        slot_condition: EffectId::NONE,
        weather: EffectId::NONE,
        terrain: EffectId::NONE,
        pseudo_weather: EffectId::NONE,
        heal: None,
        force_switch: false,
        self_switch: dex::SelfSwitch::None,
        self_effect: None,
        hooks: &[],
    };
    MoveEffectsScratch {
        base: &EMPTY,
        boosts: boosts([(Stat::Atk, -1), (Stat::SpA, -1)]),
        status: EffectId::NONE,
        volatile_status: EffectId::NONE,
        side_condition: EffectId::NONE,
        slot_condition: EffectId::NONE,
        weather: EffectId::NONE,
        terrain: EffectId::NONE,
        pseudo_weather: EffectId::NONE,
        chance: None,
        heal: None,
        force_switch: false,
        self_switch: dex::SelfSwitch::None,
        suppressed_hooks: 0,
    }
}
