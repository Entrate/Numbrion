//! Ports data/moves.ts:7673 (grassyterrain terrain condition). No direct PRNG draws; Type events may run.
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
pub const ID: EffectId = dex::CONDITION_GRASSYTERRAIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_GRASSYTERRAIN_DURATIONCALLBACK,
    dex::HOOK_CONDITION_GRASSYTERRAIN_ONBASEPOWER,
    dex::HOOK_CONDITION_GRASSYTERRAIN_ONRESIDUAL,
    dex::HOOK_CONDITION_GRASSYTERRAIN_ONFIELDSTART,
    dex::HOOK_CONDITION_GRASSYTERRAIN_ONFIELDEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:7686-7691. `source?.hasItem('terrainextender') ? 8 : 5`; direct call. PRNG: none.
        dex::HOOK_CONDITION_GRASSYTERRAIN_DURATIONCALLBACK => {
            support::duration_callback(b, cx, dex::key_ids!("terrainextender"))
        }
        // data/moves.ts:7693-7703 (priority 6). Earthquake (Bulldoze/Magnitude outside scope) on grounded, not semi-invulnerable defender: chainModify(0.5); else Grass move from grounded attacker: chainModify([5325, 4096]). PRNG: none directly.
        dex::HOOK_CONDITION_GRASSYTERRAIN_ONBASEPOWER => base_power(b, cx),
        // data/moves.ts:7713-7719 (order 5, subOrder 2). Grounded, not semi-invulnerable: heal(baseMaxhp / 16, pokemon, pokemon). PRNG: none directly (heal events).
        dex::HOOK_CONDITION_GRASSYTERRAIN_ONRESIDUAL => residual(b, cx),
        // data/moves.ts:7704-7710. -fieldstart move: Grassy Terrain (+ [from] ability / [of] source). PRNG: none.
        dex::HOOK_CONDITION_GRASSYTERRAIN_ONFIELDSTART => {
            support::terrain_field_start(b, cx, "move: Grassy Terrain");
            Relay::Undefined
        }
        // data/moves.ts:7722-7724. -fieldend move: Grassy Terrain. PRNG: none.
        dex::HOOK_CONDITION_GRASSYTERRAIN_ONFIELDEND => {
            support::terrain_end(b, "move: Grassy Terrain");
            Relay::Undefined
        }
        _ => panic!("unexpected grassyterrain hook"),
    }
}

fn base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let attacker = mon_arg(b, cx, 1);
    let defender = mon_arg(b, cx, 2);
    // weakenedMoves = ['earthquake', 'bulldoze', 'magnitude']; only Earthquake is scoped.
    let id = b.active_move(MoveHandle(move_arg(b, cx, 3))).id;
    if id == dex::MOVE_EARTHQUAKE
        && support::grounded(b, defender)
        && !b.is_semi_invulnerable(defender)
    {
        b.chain_modify(0.5, 1.0);
        return Relay::Undefined;
    }
    if support::move_type(b, cx, 3) == support::TYPE_GRASS && support::grounded(b, attacker) {
        b.chain_modify(5325.0, 4096.0);
    }
    Relay::Undefined
}
fn residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    if support::grounded(b, m) && !b.is_semi_invulnerable(m) {
        let amount = b.state.pokemon[m.0 as usize].max_hp as f64 / 16.0;
        b.heal(amount, Some(m), Some(m), HealEffect::Context);
    }
    // else: `this.debug(...)` produces no protocol output.
    Relay::Undefined
}
