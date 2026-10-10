//! Ports data/conditions.ts:324-363 (`choicelock`, added by Choice Band/Scarf/Specs' onModifyMove).
//! Payload: word 0 = `effectState.move` (raw EffectId, present bit 8; see `locktrap/mod.rs`).
//! Direct PRNG draws: none. Nested draws: `removeVolatile` events, `disableMove` none.
//! This file also hosts the batch-private helpers in `locktrap/` (sibling files use
//! `crate::effects::registry::conditions_choicelock::support`).
use crate::{
    Battle,
    actions::MoveHandle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg, move_overlay},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, MoveLineEdit},
    state::scratch::move_runtime as rt,
};
#[path = "locktrap/mod.rs"]
pub(super) mod support;
pub const ID: EffectId = dex::CONDITION_CHOICELOCK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_CHOICELOCK_ONSTART,
    dex::HOOK_CONDITION_CHOICELOCK_ONBEFOREMOVE,
    dex::HOOK_CONDITION_CHOICELOCK_ONDISABLEMOVE,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:327-331 onStart(pokemon). PRNG: none.
        // `if (!this.activeMove) throw`; falsy id / hasBounced / sourceEffect === 'snatch' -> return false
        // (addVolatile then drops the volatile). `sourceEffect === 'snatch'` is unreachable: Snatch is not in
        // the scoped dex, so no ActiveMove can carry that id. Otherwise `effectState.move = activeMove.id`.
        dex::HOOK_CONDITION_CHOICELOCK_ONSTART => {
            let handle = b.scratch.active_move;
            assert!(handle != MoveHandle::NONE, "Battle.activeMove is null");
            let (id, has_bounced) = {
                let m = b.active_move(handle);
                (m.id, m.runtime_flags & rt::HAS_BOUNCED != 0)
            };
            if id == EffectId::NONE || has_bounced {
                return Relay::FAIL;
            }
            support::store_move(b, cx, id);
            Relay::Undefined
        }
        // data/conditions.ts:332-346 onBeforeMove(pokemon, target, move). PRNG: none.
        // Not holding a Choice item: removeVolatile('choicelock') (End event), return undefined.
        // Otherwise, unless the item is ignored (Dynamax is outside the format) and the move is neither the
        // locked move nor Struggle: `|move|pokemon|name` (addMove), `[still]`, `|-fail|pokemon`, return false
        // (no PP is lost, the move never starts).
        dex::HOOK_CONDITION_CHOICELOCK_ONBEFOREMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            if !support::holds_choice_item(b, pokemon) {
                b.remove_volatile(pokemon, dex::CONDITION_CHOICELOCK);
                return Relay::Undefined;
            }
            let mv = move_arg(b, cx, 2);
            let move_id = move_overlay(b, mv).id;
            if !b.ignoring_item(pokemon)
                && move_id != support::stored_move(b, cx)
                && move_id != dex::MOVE_STRUGGLE
            {
                b.add_move(LogEntry::new(
                    "move",
                    &[
                        LogArg::Mon(pokemon),
                        LogArg::Effect(EffectRef::ActiveMove(mv)),
                    ],
                    &[],
                ));
                b.attr_last_move(MoveLineEdit::Still);
                // this.debug("Disabled by Choice item lock") produces no protocol output.
                b.add(LogEntry::new("-fail", &[LogArg::Mon(pokemon)], &[]));
                return Relay::FAIL;
            }
            Relay::Undefined
        }
        // data/conditions.ts:347-362 onDisableMove(pokemon). PRNG: none.
        // Lost item / missing locked move: removeVolatile and stop. Ignored item: stop. Otherwise disable every
        // other slot with `disableMove(id, false, effectState.sourceEffect)` (the item that added the lock).
        dex::HOOK_CONDITION_CHOICELOCK_ONDISABLEMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            let locked = support::stored_move(b, cx);
            if !support::holds_choice_item(b, pokemon) || !b.has_move(pokemon, locked).truthy() {
                b.remove_volatile(pokemon, dex::CONDITION_CHOICELOCK);
                return Relay::Undefined;
            }
            if b.ignoring_item(pokemon) {
                return Relay::Undefined;
            }
            let source_effect = support::state_source_effect(b, cx);
            let (ids, len) = support::move_slot_ids(b, pokemon);
            for &id in &ids[..len] {
                if id != locked {
                    b.disable_move(pokemon, id, false, source_effect);
                }
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Choice lock function site"),
    }
}
