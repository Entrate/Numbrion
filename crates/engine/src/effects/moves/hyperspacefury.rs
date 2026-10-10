//! Ports data/moves.ts:9160-9192 (Hyperspace Fury): the `onTry` Hoopa-Unbound gate. The
//! `self.boosts` drop (-1 Def) is declarative data handled by the core. No PRNG draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
};
pub const ID: EffectId = dex::MOVE_HYPERSPACEFURY;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_HYPERSPACEFURY_ONTRY];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_HYPERSPACEFURY_ONTRY => on_try(b, cx),
        _ => panic!("unexpected Hyperspace Fury function site"),
    }
}

// data/moves.ts:9170-9183 onTry(source): Try single event (battle-actions.ts:586), args
// [source, target, move].
//   if (source.species.name === 'Hoopa-Unbound') return;                     // undefined
//   this.hint("Only a Pokemon whose form is Hoopa Unbound can use this move.");
//   if (source.species.name === 'Hoopa') {
//     this.attrLastMove('[still]');
//     this.add('-fail', source, 'move: Hyperspace Fury', '[forme]');
//     return null;
//   }
//   this.attrLastMove('[still]');
//   this.add('-fail', source, 'move: Hyperspace Fury');
//   return null;
// `species.name` is the current species (BattleState species id; a Transform/forme change moves
// it). PRNG: none.
fn on_try<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let source = mon_arg(b, cx, 0);
    let species = b.state.pokemon[source.0 as usize].species;
    if species == dex::SPECIES_HOOPAUNBOUND {
        return Relay::Undefined;
    }
    b.hint(
        LogArg::Text("Only a Pokemon whose form is Hoopa Unbound can use this move."),
        false,
        None,
    );
    if species == dex::SPECIES_HOOPA {
        b.attr_last_move(MoveLineEdit::Still);
        b.add(LogEntry::new(
            "-fail",
            &[LogArg::Mon(source), LogArg::Text("move: Hyperspace Fury")],
            &[LogTag::Bare("forme")],
        ));
        return Relay::Null;
    }
    b.attr_last_move(MoveLineEdit::Still);
    b.add(LogEntry::new(
        "-fail",
        &[LogArg::Mon(source), LogArg::Text("move: Hyperspace Fury")],
        &[],
    ));
    Relay::Null
}
