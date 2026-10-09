//! BattleQueue port. OWNER L. Priority comparison and tie sorting belong to event/.
#![allow(unused_variables)]

use crate::{
    Battle,
    event::EffectRef,
    ids::{EffectId, EventId, MonId},
    log::LogSink,
    state::choices::{Action, ActionKind, ChosenMoveKind},
};

/// Unresolved ActionChoice (`sim/battle-queue.ts:115-120,166-277`).
/// None target_loc preserves JS undefined; explicit zero means random target.
/// PRNG: constructing this value draws nothing; resolve_action may draw.
#[derive(Clone, Copy, Debug)]
pub struct ActionChoice {
    pub kind: ActionKind,
    pub pokemon: MonId,
    pub target: MonId,
    pub move_id: EffectId,
    pub move_kind: ChosenMoveKind,
    pub target_loc: Option<i8>,
    pub source_effect: EffectRef,
    pub terastallize: bool,
    pub event: Option<EventId>,
}

/// Gen9 expansion: beforeTurnMove, Tera, priorityChargeMove and main move.
/// `sim/battle-queue.ts:197-244`; PRNG: only resolving entries draws.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedActions {
    pub entries: [Action; 4],
    pub len: u8,
}

impl Default for ResolvedActions {
    fn default() -> Self {
        Self {
            entries: [Action::default(); 4],
            len: 0,
        }
    }
}

/// Ephemeral index for a retrieved action (`sim/battle-queue.ts:315-365`).
/// Invalid after any queue mutation; copy the Action before calling events.
/// PRNG: none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct QueueHandle(pub u8);

impl<L: LogSink> Battle<L> {
    /// Fill priority/speed/targets and expand actions; battle-queue.ts:166-277.
    /// PRNG: recursive pre-actions, FractionalPriority, random targets, then speed events.
    pub fn resolve_action(&mut self, choice: ActionChoice, mid_turn: bool) -> ResolvedActions {
        todo!("L: resolveAction, preserving recursive resolution/draw order")
    }

    /// Append resolved choices; battle-queue.ts:307-313.
    /// PRNG: resolve_action only; no independent sort or insertion roll.
    pub fn queue_add_choice(&mut self, choice: ActionChoice) {
        todo!("L: addChoice")
    }

    /// Insert without resorting existing actions; battle-queue.ts:369-403.
    /// PRNG: updateSpeed/resolve_action; one random(first,last+1) for a tied interval.
    pub fn queue_insert_choice(&mut self, choice: ActionChoice, mid_turn: bool) {
        todo!("L: insertChoice")
    }

    /// Selection-sort queue with exact tie shuffles; battle-queue.ts:418-422.
    /// PRNG: event::speed_sort draws per Fisher-Yates step on ties, including no-op swaps.
    pub fn queue_sort(&mut self) {
        todo!("L: sort using the event owner's comparator and speedSort")
    }

    /// Prioritize existing or newly resolved action; battle-queue.ts:282-293.
    /// A handle removes that existing entry; None inserts a fresh copied action.
    /// PRNG: none. Assign sourceEffect, order=3 and unshift exactly once.
    pub fn queue_prioritize_action(
        &mut self,
        action: Action,
        existing: Option<QueueHandle>,
        source_effect: EffectRef,
    ) {
        todo!("L: prioritizeAction")
    }

    /// Cancel actor actions and insert replacement; battle-queue.ts:301-305.
    /// PRNG: queue_insert_choice and its nested events/insertion tie roll.
    pub fn queue_change_action(&mut self, pokemon: MonId, choice: ActionChoice) {
        todo!("L: changeAction")
    }

    /// Earliest move/switch/instaswitch; battle-queue.ts:315-322.
    /// PRNG: none; returned index is invalid after a queue mutation.
    pub fn queue_will_act(&self) -> Option<QueueHandle> {
        todo!("L: willAct")
    }

    /// Actor's next move, unless fainted; battle-queue.ts:324-332.
    /// PRNG: none; returned index is invalid after a queue mutation.
    pub fn queue_will_move(&self, pokemon: MonId) -> Option<QueueHandle> {
        todo!("L: willMove")
    }

    /// Actor's switch/instaswitch; battle-queue.ts:355-365.
    /// PRNG: none; returned index is invalid after a queue mutation.
    pub fn queue_will_switch(&self, pokemon: MonId) -> Option<QueueHandle> {
        todo!("L: willSwitch")
    }

    /// Remove all actor actions; battle-queue.ts:334-343. PRNG: none.
    pub fn queue_cancel_action(&mut self, pokemon: MonId) -> bool {
        todo!("L: cancelAction")
    }

    /// Remove first actor move only; battle-queue.ts:345-353. PRNG: none.
    pub fn queue_cancel_move(&mut self, pokemon: MonId) -> bool {
        todo!("L: cancelMove")
    }

    /// Copy a retrieved action; battle-queue.ts:154-157,315-365. PRNG: none.
    pub fn queue_action(&self, handle: QueueHandle) -> Action {
        todo!("L: checked ephemeral action lookup")
    }

    /// Remove first action, shifting remaining entries; battle-queue.ts:143-145.
    /// PRNG: none.
    pub fn queue_shift(&mut self) -> Option<Action> {
        todo!("L: shift")
    }

    /// Inspect first/last action; battle-queue.ts:146-148. PRNG: none.
    pub fn queue_peek(&self, end: bool) -> Option<Action> {
        todo!("L: peek")
    }

    /// Append a resolved action; battle-queue.ts:149-151. PRNG: none.
    pub fn queue_push(&mut self, action: Action) {
        todo!("L: push with capacity guard")
    }

    /// Prepend a resolved action; battle-queue.ts:152-154. PRNG: none.
    pub fn queue_unshift(&mut self, action: Action) {
        todo!("L: unshift with capacity guard")
    }

    /// Empty queue; battle-queue.ts:405-407. PRNG: none.
    pub fn queue_clear(&mut self) {
        todo!("L: clear")
    }

    /// Mutate priority and action speed; sim/battle.ts:2623-2667.
    /// PRNG: getTarget may sample; ModifyPriority/stat handlers may tie-shuffle/draw.
    pub fn get_action_speed(&mut self, action: &mut Action) {
        todo!("L: getActionSpeed")
    }
}
impl<L: LogSink> Battle<L> {
    /// Mutable action retrieved by willMove/willSwitch (battle-queue.ts:324-365;
    /// data/abilities.ts:575-580). PRNG: none; drop this borrow before callbacks.
    pub fn queue_action_mut(&mut self, _handle: QueueHandle) -> &mut Action {
        todo!("L: mutable ephemeral queue entry")
    }
}
