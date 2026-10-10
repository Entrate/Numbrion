//! Ports data/moves.ts:3945-3969 (Double Shock): the `onTryMove` gate and the `self.onHit`
//! type change. No PRNG draws; Type events dispatched by `hasType`/`getTypes` may sort ties.
//!
//! Function sites: `onTryMove` (move callback) and `self.onHit` (the self-effect's own Hit
//! callback, run by the self-drop `moveHit(pokemon, pokemon, move, move.self, ...)`).
use crate::{
    Battle,
    actions::Types,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
};
#[path = "movecallbacks/mod.rs"]
mod support;
pub const ID: EffectId = dex::MOVE_DOUBLESHOCK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_DOUBLESHOCK_ONTRYMOVE,
    dex::HOOK_MOVE_DOUBLESHOCK_SELF_ONHIT,
];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_DOUBLESHOCK_ONTRYMOVE => on_try_move(b, cx),
        dex::HOOK_MOVE_DOUBLESHOCK_SELF_ONHIT => self_on_hit(b, cx),
        _ => panic!("unexpected Double Shock function site"),
    }
}

// data/moves.ts:3954-3959 onTryMove(pokemon, target, move): singleEvent('TryMove', move, null,
// pokemon, target, move) (battle-actions.ts:486), args [pokemon, target, move].
//   if (pokemon.hasType('Electric')) return;            // undefined: the move proceeds
//   this.add('-fail', pokemon, 'move: Double Shock');
//   this.attrLastMove('[still]');
//   return null;                                        // silent fail
// PRNG: none directly.
fn on_try_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    if b.has_type(pokemon, &[support::TYPE_ELECTRIC]) {
        return Relay::Undefined;
    }
    b.add(LogEntry::new(
        "-fail",
        &[LogArg::Mon(pokemon), LogArg::Text("move: Double Shock")],
        &[],
    ));
    b.attr_last_move(MoveLineEdit::Still);
    Relay::Null
}

// data/moves.ts:3961-3964 self.onHit(pokemon): Hit single event of the self effect, args
// [pokemon, pokemon, move].
//   pokemon.setType(pokemon.getTypes(true).map(type => type === "Electric" ? "???" : type));
//   this.add('-start', pokemon, 'typechange', pokemon.getTypes().join('/'),
//            '[from] move: Double Shock');
// setType (enforce = false) is refused for a Terastallized user, in which case getTypes() is the
// Tera type and the line is still emitted. Returns undefined. PRNG: none directly.
fn self_on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let mut types = b.get_types(pokemon, true, false);
    for ty in &mut types.values[..types.len as usize] {
        if *ty == support::TYPE_ELECTRIC {
            *ty = support::TYPE_QUESTION;
        }
    }
    b.set_type(pokemon, types, false);
    let current = b.get_types(pokemon, false, false);
    let joined = join_types(&current);
    b.add(LogEntry::new(
        "-start",
        &[
            LogArg::Mon(pokemon),
            LogArg::Text("typechange"),
            LogArg::Parts(&joined[..usize::from(current.len) * 2 - 1]),
        ],
        &[LogTag::From(EffectRef::Dex(dex::MOVE_DOUBLESHOCK))],
    ));
    Relay::Undefined
}

/// `types.join('/')` as five stack parts (at most three types: two plus an added type).
fn join_types(types: &Types) -> [LogArg<'static>; 5] {
    let mut parts = [LogArg::Empty; 5];
    for i in 0..usize::from(types.len) {
        if i > 0 {
            parts[2 * i - 1] = LogArg::Text("/");
        }
        parts[2 * i] = LogArg::Type(types.values[i]);
    }
    parts
}
