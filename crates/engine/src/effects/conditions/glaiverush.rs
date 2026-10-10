//! Ports the Glaive Rush volatile, data/moves.ts:6659-6675 (the `condition` of the move at
//! 6647-6678; the move itself only sets `self.volatileStatus: 'glaiverush'`).
//! No PRNG draws.
//!
//! The volatile has no per-instance state: `noCopy` is declarative metadata and nothing reads
//! `effectState`, so no payload words are used.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
};
pub const ID: EffectId = dex::CONDITION_GLAIVERUSH;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_GLAIVERUSH_ONSTART,
    dex::HOOK_CONDITION_GLAIVERUSH_ONACCURACY,
    dex::HOOK_CONDITION_GLAIVERUSH_ONSOURCEMODIFYDAMAGE,
    dex::HOOK_CONDITION_GLAIVERUSH_ONBEFOREMOVE,
];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_CONDITION_GLAIVERUSH_ONSTART => on_start(b, cx),
        dex::HOOK_CONDITION_GLAIVERUSH_ONACCURACY => on_accuracy(b, cx),
        dex::HOOK_CONDITION_GLAIVERUSH_ONSOURCEMODIFYDAMAGE => on_source_modify_damage(b, cx),
        dex::HOOK_CONDITION_GLAIVERUSH_ONBEFOREMOVE => on_before_move(b, cx),
        _ => panic!("unexpected Glaive Rush function site"),
    }
}

// data/moves.ts:6661-6663 onStart(pokemon): Start single event from addVolatile
// (pokemon.ts:2018), args [pokemon, source, sourceEffect].
//   this.add('-singlemove', pokemon, 'Glaive Rush', '[silent]');
// Returns undefined (the volatile stays). PRNG: none.
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    b.add(LogEntry::new(
        "-singlemove",
        &[LogArg::Mon(pokemon), LogArg::Text("Glaive Rush")],
        &[LogTag::Bare("silent")],
    ));
    Relay::Undefined
}

// data/moves.ts:6664-6666 onAccuracy(): runEvent('Accuracy', target, source, move, accuracy)
// (battle-actions.ts:709) with the volatile on the defender; it reads no argument and returns
// `true`, which makes every move auto-hit (hitStepAccuracy then skips the accuracy roll, so no
// randomChance draw happens). PRNG: none.
fn on_accuracy<L: LogSink>(_b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    Relay::Bool(true)
}

// data/moves.ts:6667-6669 onSourceModifyDamage(): runEvent('ModifyDamage', pokemon, target, move,
// baseDamage) (battle-actions.ts:1795) with the volatile on the defender (the `Source` prefix).
// `return this.chainModify(2)` mutates the event modifier and returns undefined. PRNG: none.
fn on_source_modify_damage<L: LogSink>(b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    b.chain_modify(2.0, 1.0);
    Relay::Undefined
}

// data/moves.ts:6670-6674 onBeforeMove(pokemon) at priority 100: runEvent('BeforeMove', pokemon,
// target, move) (battle-actions.ts via runMove), args [pokemon, target, move].
//   this.debug('removing Glaive Rush drawback before attack');   // no output
//   pokemon.removeVolatile('glaiverush');                          // the result is ignored
// removeVolatile runs the (absent) End hook and erases this very cell, which stays pinned while
// the handler runs. Returns undefined. PRNG: none.
fn on_before_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    b.remove_volatile(pokemon, dex::CONDITION_GLAIVERUSH);
    Relay::Undefined
}
