//! Private helpers shared by the `charge_priority_multihit` batch (not an effect: build.rs only
//! discovers direct `.rs` children of the family directories). Declared once, in
//! `moves/electroshot.rs`, as `moves_electroshot::cp`; the other files of the batch import it from there.
//! Nothing here allocates, formats strings or draws from the PRNG on its own.
use crate::{
    Battle,
    actions::{Attribution, MoveHandle},
    dex,
    effects::support::*,
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, MoveLineEdit},
    state::scratch::OrderedBoosts,
};

// ---------------------------------------------------------------------------------------------
// Compile-time type ids (the generated TYPE_NAMES table is 0-based; TypeId 0 is "absent").
// ---------------------------------------------------------------------------------------------

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
pub const fn type_named(name: &str) -> TypeId {
    let mut i = 0;
    while i < dex::TYPE_NAMES.len() {
        if same(dex::TYPE_NAMES[i], name) {
            return TypeId(i as u8 + 1);
        }
        i += 1;
    }
    panic!("unknown type name")
}
pub const TYPE_ELECTRIC: TypeId = type_named("Electric");
pub const TYPE_FLYING: TypeId = type_named("Flying");

// ---------------------------------------------------------------------------------------------
// Argument access.
// ---------------------------------------------------------------------------------------------

/// The live ActiveMove named by argument `index`, or `None` when the callback received
/// undefined/null there (`move?.` in the source).
pub fn live_move_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> Option<u8> {
    match b.event_arg(cx, index) {
        EventArg::Move(i)
        | EventArg::Effect(EffectRef::ActiveMove(i))
        | EventArg::Relay(Relay::ActiveMove(i)) => Some(i),
        _ => None,
    }
}

/// A JS Pokemon argument as a holder argument (for run_event targets/sources).
pub fn holder(m: MonId) -> EventArg {
    EventArg::Holder(Holder::mon(m))
}

/// `{ spa: 1 }` (boost index: atk 0, def 1, spa 2, spd 3, spe 4).
pub fn spa_plus_one() -> OrderedBoosts {
    let mut o = OrderedBoosts::default();
    o.values[2] = 1;
    o.order[0] = 2;
    o.len = 1;
    o.present = 1 << 2;
    o
}

// ---------------------------------------------------------------------------------------------
// Sucker Punch / Thunderclap.
// ---------------------------------------------------------------------------------------------

/// `me first`: not in the generated scope, so `NONE` (which never equals a live move id).
const MEFIRST: EffectId = optional_id(dex::MOVES_DATA, "mefirst");

/// onTry(source, target) of Sucker Punch (data/moves.ts:18405-18412) and Thunderclap
/// (19506-19513), identical statement for statement:
///
/// ```text
/// const action = this.queue.willMove(target);
/// const move = action?.choice === 'move' ? action.move : null;
/// if (!move || (move.category === 'Status' && move.id !== 'mefirst') || target.volatiles['mustrecharge']) return false;
/// ```
///
/// `queue.willMove` only ever returns a `choice === 'move'` action (a fainted target has none),
/// so `move` is null exactly when no move action is queued for the target. The queued move is the
/// ActiveMove built in resolveAction, whose category is still the dex one (ModifyMove has not
/// run). The synthetic Recharge move is not a real move: its category is not 'Status', and the
/// target necessarily has `mustrecharge`, so the move fails either way. PRNG: none.
pub fn sucker_punch_try<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 1);
    let blocked = match b.queue_will_move(target) {
        None => true,
        Some(handle) => {
            let queued = b.queue_action(handle).move_data;
            queued.kind == crate::state::choices::ChosenMoveKind::Dex
                && dex::move_data(queued.id).category == dex::Category::Status
                && queued.id != MEFIRST
        }
    };
    if blocked
        || b.get_volatile(target, dex::CONDITION_MUSTRECHARGE)
            .is_some()
    {
        return Relay::FAIL;
    }
    Relay::Undefined
}

// ---------------------------------------------------------------------------------------------
// The shared two-turn "charge" TryMove body.
// ---------------------------------------------------------------------------------------------

/// onTryMove(attacker, defender, move) of Electro Shot (data/moves.ts:4640-4658), Meteor Beam
/// (11749-11760), Phantom Force (13318-13329), Shadow Force (16084-16095) and Solar Beam
/// (17234-17252). The five bodies are the same statement sequence; the only differences are the
/// optional `this.boost({ spa: 1 }, ...)` (Electro Shot, Meteor Beam) and the optional weather
/// short-cut (`skip_weather`: Electro Shot -> rain, Solar Beam -> sun), which sit at the same
/// position in every move.
///
/// Statement order (kept exactly):
/// 1. `attacker.removeVolatile(move.id)` -> second turn: return undefined (the move fires).
/// 2. `-prepare|attacker|move.name`.
/// 3. optional `boost({spa: 1}, attacker, attacker, move)`.
/// 4. optional weather short-cut: `-anim` (with `[still]` on the move line) and return undefined.
/// 5. `runEvent('ChargeMove', attacker, defender, move)` falsy (Power Herb) -> return undefined.
/// 6. `attacker.addVolatile('twoturnmove', defender)`; `return null`.
///
/// PRNG: none directly. Nested: boost events and `addVolatile` (twoturnmove Start) may speed-sort
/// ties; `twoturnmove` Start draws `sample` only for a fainted-target Metronome-style call.
pub fn charge_try_move<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    boost_spa: bool,
    skip_weather: Option<EffectId>,
) -> Relay {
    let attacker = mon_arg(b, cx, 0);
    let defender = mon_arg(b, cx, 1);
    let mv = move_arg(b, cx, 2);
    let move_id = move_overlay(b, mv).id;
    let effect = EffectRef::ActiveMove(mv);
    if b.remove_volatile(attacker, move_id) {
        return Relay::Undefined;
    }
    b.add(LogEntry::new(
        "-prepare",
        &[LogArg::Mon(attacker), LogArg::Effect(effect)],
        &[],
    ));
    if boost_spa {
        b.boost(
            spa_plus_one(),
            Some(attacker),
            Attribution::from_move(attacker, effect),
            false,
            false,
        );
    }
    if let Some(weather) = skip_weather {
        if b.effective_weather(attacker) == weather {
            b.attr_last_move(MoveLineEdit::Still);
            b.add_move(LogEntry::new(
                "-anim",
                &[
                    LogArg::Mon(attacker),
                    LogArg::Effect(effect),
                    LogArg::Mon(defender),
                ],
                &[],
            ));
            return Relay::Undefined;
        }
    }
    if !b
        .prepare_charge_move(attacker, holder(defender), MoveHandle(mv))
        .truthy()
    {
        return Relay::Undefined;
    }
    b.add_volatile(
        attacker,
        dex::CONDITION_TWOTURNMOVE,
        Attribution {
            source: holder(defender),
            effect: EffectRef::None,
        },
        None,
    );
    Relay::Null
}

#[cfg(test)]
mod tests;
