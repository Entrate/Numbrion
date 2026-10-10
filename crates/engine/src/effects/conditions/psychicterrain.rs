//! Ports data/moves.ts:14095 (psychicterrain terrain condition). No direct PRNG draws; Type events may run.
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
pub const ID: EffectId = dex::CONDITION_PSYCHICTERRAIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_PSYCHICTERRAIN_DURATIONCALLBACK,
    dex::HOOK_CONDITION_PSYCHICTERRAIN_ONTRYHIT,
    dex::HOOK_CONDITION_PSYCHICTERRAIN_ONBASEPOWER,
    dex::HOOK_CONDITION_PSYCHICTERRAIN_ONFIELDSTART,
    dex::HOOK_CONDITION_PSYCHICTERRAIN_ONFIELDEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:14108-14113. `source?.hasItem('terrainextender') ? 8 : 5`; direct call. PRNG: none.
        dex::HOOK_CONDITION_PSYCHICTERRAIN_DURATIONCALLBACK => {
            support::duration_callback(b, cx, dex::key_ids!("terrainextender"))
        }
        // data/moves.ts:14115-14129 (priority 4). Blocks priority > 0.1, non-self-target moves against grounded, non-ally, not semi-invulnerable targets: -activate, return null; hint when an airborne target dodges a priority move. PRNG: none directly.
        dex::HOOK_CONDITION_PSYCHICTERRAIN_ONTRYHIT => try_hit(b, cx),
        // data/moves.ts:14131-14136 (priority 6). Psychic move from a grounded, not semi-invulnerable attacker: chainModify([5325, 4096]). PRNG: none directly.
        dex::HOOK_CONDITION_PSYCHICTERRAIN_ONBASEPOWER => base_power(b, cx),
        // data/moves.ts:14137-14143. -fieldstart move: Psychic Terrain (+ [from] ability / [of] source). PRNG: none.
        dex::HOOK_CONDITION_PSYCHICTERRAIN_ONFIELDSTART => {
            support::terrain_field_start(b, cx, "move: Psychic Terrain");
            Relay::Undefined
        }
        // data/moves.ts:14146-14148. -fieldend move: Psychic Terrain. PRNG: none.
        dex::HOOK_CONDITION_PSYCHICTERRAIN_ONFIELDEND => {
            support::terrain_end(b, "move: Psychic Terrain");
            Relay::Undefined
        }
        _ => panic!("unexpected psychicterrain hook"),
    }
}

fn try_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = support::opt_mon(b, cx, 1);
    let effect = support::effect_ref(b, cx, 2);
    if effect != EffectRef::None {
        let (priority, move_target) = support::move_priority_target(b, effect);
        if priority <= 0.1 || move_target == MoveTarget::SelfTarget {
            return Relay::Undefined;
        }
    }
    if b.is_semi_invulnerable(target) || source.is_some_and(|s| s.side() == target.side()) {
        return Relay::Undefined;
    }
    if !support::grounded(b, target) {
        // `this.dex.moves.get(effect.id)` throws for a missing effect in the source.
        let id = b.event_effect_id(effect);
        assert!(
            id.kind() == Some(EffectKind::Move),
            "Psychic Terrain TryHit effect is a move"
        );
        if dex::move_data(id).priority > 0 {
            b.hint(
                LogArg::Text("Psychic Terrain doesn't affect airborne Pok\u{e9}mon."),
                false,
                None,
            );
        }
        return Relay::Undefined;
    }
    b.add(LogEntry::new(
        "-activate",
        &[LogArg::Mon(target), LogArg::Text("move: Psychic Terrain")],
        &[],
    ));
    Relay::Null
}
fn base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let attacker = mon_arg(b, cx, 1);
    if support::move_type(b, cx, 3) == support::TYPE_PSYCHIC
        && support::grounded(b, attacker)
        && !b.is_semi_invulnerable(attacker)
    {
        b.chain_modify(5325.0, 4096.0);
    }
    Relay::Undefined
}
