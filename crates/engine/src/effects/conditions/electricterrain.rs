//! Ports data/moves.ts:4497 (electricterrain terrain condition). No direct PRNG draws; Type events may run.
#![allow(unused_imports)]
use crate::effects::registry::abilities_airlock::support;
use crate::{
    Battle,
    actions::{Attribution, HealEffect, MoveHandle},
    dex::{self, HookId, ImmunityId, MoveTarget},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::scratch::MoveAccuracy,
};
pub const ID: EffectId = dex::CONDITION_ELECTRICTERRAIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_ELECTRICTERRAIN_DURATIONCALLBACK,
    dex::HOOK_CONDITION_ELECTRICTERRAIN_ONSETSTATUS,
    dex::HOOK_CONDITION_ELECTRICTERRAIN_ONTRYADDVOLATILE,
    dex::HOOK_CONDITION_ELECTRICTERRAIN_ONBASEPOWER,
    dex::HOOK_CONDITION_ELECTRICTERRAIN_ONFIELDSTART,
    dex::HOOK_CONDITION_ELECTRICTERRAIN_ONFIELDEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:4510-4515. `source?.hasItem('terrainextender') ? 8 : 5`; direct call. PRNG: none.
        dex::HOOK_CONDITION_ELECTRICTERRAIN_DURATIONCALLBACK => {
            support::duration_callback(b, cx, "terrainextender")
        }
        // data/moves.ts:4516-4523. slp on grounded, not semi-invulnerable target: -activate when effect is yawn or a secondary-less Move; return false. PRNG: none directly.
        dex::HOOK_CONDITION_ELECTRICTERRAIN_ONSETSTATUS => set_status(b, cx),
        // data/moves.ts:4524-4530. grounded, not semi-invulnerable target and status yawn: -activate; return null. PRNG: none directly.
        dex::HOOK_CONDITION_ELECTRICTERRAIN_ONTRYADDVOLATILE => try_add_volatile(b, cx),
        // data/moves.ts:4532-4537 (priority 6). Electric move from a grounded, not semi-invulnerable attacker: chainModify([5325, 4096]). PRNG: none directly.
        dex::HOOK_CONDITION_ELECTRICTERRAIN_ONBASEPOWER => base_power(b, cx),
        // data/moves.ts:4538-4544. -fieldstart move: Electric Terrain (+ [from] ability / [of] source). PRNG: none.
        dex::HOOK_CONDITION_ELECTRICTERRAIN_ONFIELDSTART => {
            support::terrain_field_start(b, cx, "move: Electric Terrain");
            Relay::Undefined
        }
        // data/moves.ts:4547-4549. -fieldend move: Electric Terrain. PRNG: none.
        dex::HOOK_CONDITION_ELECTRICTERRAIN_ONFIELDEND => {
            support::terrain_end(b, "move: Electric Terrain");
            Relay::Undefined
        }
        _ => panic!("unexpected electricterrain hook"),
    }
}

fn set_status<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let status = support::effect_id(b, cx, 0);
    let target = mon_arg(b, cx, 1);
    if status == dex::CONDITION_SLP
        && support::grounded(b, target)
        && !b.is_semi_invulnerable(target)
    {
        let effect = support::effect_ref(b, cx, 3);
        let id = b.event_effect_id(effect);
        if id == dex::CONDITION_YAWN
            || id == dex::MOVE_YAWN
            || (effect != EffectRef::None
                && b.event_effect_type(effect) == dex::EffectType::Move
                && !support::has_secondaries(b, effect))
        {
            b.add(LogEntry::new(
                "-activate",
                &[LogArg::Mon(target), LogArg::Text("move: Electric Terrain")],
                &[],
            ));
        }
        return Relay::Bool(false);
    }
    Relay::Undefined
}
fn try_add_volatile<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 1);
    if !support::grounded(b, target) || b.is_semi_invulnerable(target) {
        return Relay::Undefined;
    }
    if support::effect_id(b, cx, 0) == dex::CONDITION_YAWN {
        b.add(LogEntry::new(
            "-activate",
            &[LogArg::Mon(target), LogArg::Text("move: Electric Terrain")],
            &[],
        ));
        return Relay::Null;
    }
    Relay::Undefined
}
fn base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let attacker = mon_arg(b, cx, 1);
    if support::move_type(b, cx, 3) == support::TYPE_ELECTRIC
        && support::grounded(b, attacker)
        && !b.is_semi_invulnerable(attacker)
    {
        b.chain_modify(5325.0, 4096.0);
    }
    Relay::Undefined
}
