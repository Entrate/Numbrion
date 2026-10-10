//! Ports data/moves.ts:2498-2526 (Clangorous Soul). No direct PRNG draws; the boost and damage
//! mutators dispatch events whose listeners may speed-sort ties.
use crate::{
    Battle,
    actions::{Attribution, MoveHandle},
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
    state::scratch::OrderedBoosts,
};
pub const ID: EffectId = dex::MOVE_CLANGOROUSSOUL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_CLANGOROUSSOUL_ONTRY,
    dex::HOOK_MOVE_CLANGOROUSSOUL_ONTRYHIT,
    dex::HOOK_MOVE_CLANGOROUSSOUL_ONHIT,
];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_CLANGOROUSSOUL_ONTRY => on_try(b, cx),
        dex::HOOK_MOVE_CLANGOROUSSOUL_ONTRYHIT => on_try_hit(b, cx),
        dex::HOOK_MOVE_CLANGOROUSSOUL_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Clangorous Soul function site"),
    }
}

// data/moves.ts:2507-2509 onTry(source): Try single event (battle-actions.ts:586), args
// [source, target, move]. `source.hp <= (source.maxhp * 33 / 100) || source.maxhp === 1` fails
// with false (the generic `-fail` line comes from trySpreadMoveHit). Else undefined.
// PRNG: none.
fn on_try<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let source = mon_arg(b, cx, 0);
    let p = &b.state.pokemon[source.0 as usize];
    if f64::from(p.hp) <= f64::from(p.max_hp) * 33.0 / 100.0 || p.max_hp == 1 {
        return Relay::Bool(false);
    }
    Relay::Undefined
}

// data/moves.ts:2510-2513 onTryHit(pokemon, target, move): single event from the self-target
// move path (battle-actions.ts:1035), args [target, source, move].
//   if (!this.boost(move.boosts!)) return null;   // target/source/effect default from the event
//   delete move.boosts;                           // the later primary-boost step sees no boosts
// An empty OrderedBoosts (len 0) is the "deleted" overlay, so static dex boosts do not return.
// PRNG: none directly (boost events may sort ties).
fn on_try_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let handle = MoveHandle(move_arg(b, cx, 2));
    let boosts = b.active_move(handle).effects.boosts;
    if !b
        .boost(boosts, None, Attribution::DEFAULT, false, false)
        .truthy()
    {
        return Relay::Null;
    }
    b.active_move_mut(handle).effects.boosts = OrderedBoosts::default();
    Relay::Undefined
}

// data/moves.ts:2514-2516 onHit(pokemon): Hit single event, args [target, source, move].
// `this.directDamage(pokemon.maxhp * 33 / 100)` (target/source/effect from the event; clamped to
// at least 1 and floored inside directDamage). Returns undefined. PRNG: none.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let amount = f64::from(b.state.pokemon[pokemon.0 as usize].max_hp) * 33.0 / 100.0;
    b.direct_damage(amount, None, Attribution::DEFAULT);
    Relay::Undefined
}
