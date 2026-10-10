//! Ports data/moves.ts:5218-5236 (Fickle Beam). One PRNG draw per BasePower event.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
};
pub const ID: EffectId = dex::MOVE_FICKLEBEAM;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_FICKLEBEAM_ONBASEPOWER];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_FICKLEBEAM_ONBASEPOWER => on_base_power(b, cx),
        _ => panic!("unexpected Fickle Beam function site"),
    }
}

// data/moves.ts:5227-5233 onBasePower(basePower, pokemon): runEvent('BasePower', attacker,
// defender, move, basePower, true) passes [basePower, attacker, defender, move]
// (battle-actions.ts:1599).
//   if (this.randomChance(3, 10)) {                 // PRNG: exactly one random(10) draw per call
//     this.attrLastMove('[anim] Fickle Beam All Out');
//     this.add('-activate', pokemon, 'move: Fickle Beam');
//     return this.chainModify(2);                   // mutates the frame, returns undefined
//   }
// getDamage runs the BasePower event once per target and per hit, so each call draws once.
fn on_base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 1);
    if b.state.prng.random_chance(3, 10) {
        b.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
            "anim",
            LogArg::Text("Fickle Beam All Out"),
        )));
        b.add(LogEntry::new(
            "-activate",
            &[LogArg::Mon(pokemon), LogArg::Text("move: Fickle Beam")],
            &[],
        ));
        b.chain_modify(2.0, 1.0);
    }
    Relay::Undefined
}
