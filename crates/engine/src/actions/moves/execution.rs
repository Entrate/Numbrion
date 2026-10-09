//! Outside/inside move callers. OWNER M.
#![allow(unused_variables)]
use crate::{
    actions::{MoveHandle, MoveInput, RunMoveOptions, UseMoveOptions},
    battle::Battle,
    event::{EffectRef, EventArg, Relay},
    ids::MonId,
    log::LogSink,
};

impl<L: LogSink> Battle<L> {
    /// Ports sim/battle-actions.ts:210-353. Applies BeforeMove, PP, move-used,
    /// caller attribution, AfterMove, Dancer, faint messages, and win checks.
    /// PRNG: no own draw; targeting, events, and the inner hit pipeline draw.
    pub fn run_move(
        &mut self,
        move_input: MoveInput,
        user: MonId,
        target_loc: Option<i8>,
        source_effect: EffectRef,
        options: RunMoveOptions,
    ) -> Relay {
        todo!("stage M: outside move caller")
    }

    /// Ports sim/battle-actions.ts:365-376. Inside caller used by Sleep Talk,
    /// Nature Power, and Magic Bounce; preserves moveThisTurnResult sentinels.
    /// Target is Undefined, Null, or Holder(Pokemon), retaining defaulting order.
    /// PRNG: none directly; use_move_inner and dispatched effects may draw.
    pub fn use_move(
        &mut self,
        move_input: MoveInput,
        user: MonId,
        target: EventArg,
        source_effect: EffectRef,
        options: UseMoveOptions,
    ) -> Relay {
        todo!("stage M: inside move caller")
    }

    /// Ports sim/battle-actions.ts:377-545. Runs ModifyTarget/Type/Move in exact
    /// order, emits move, accounts for Pressure, and runs hit/self effects.
    /// PRNG: automatic retargeting plus hit-stage and effect draws, in source order.
    pub fn use_move_inner(
        &mut self,
        move_input: MoveInput,
        user: MonId,
        target: EventArg,
        source_effect: EffectRef,
        options: UseMoveOptions,
    ) -> Relay {
        todo!("stage M: inside move pipeline")
    }

    /// Ports the Dancer block in sim/battle-actions.ts:315-345. Uses raw stored
    /// speed and reverse ability effectOrder; retains the original dance target.
    /// PRNG: none for Dancer ordering; each recursive run_move may draw.
    pub fn run_dancers(&mut self, user: MonId, move_handle: MoveHandle, first_target: MonId) {
        todo!("stage M: recursive Dancer execution")
    }

    /// Ports sim/battle.ts:2733-2741. Calls the move's beforeTurnCallback directly,
    /// preserving the enclosing event frame; lifecycle owns the queued action.
    /// PRNG: target resolution and callback may draw; no additional draw.
    pub fn run_before_turn_move(
        &mut self,
        move_input: MoveInput,
        user: MonId,
        target_loc: Option<i8>,
    ) -> Relay {
        todo!("stage M: before-turn move callback")
    }

    /// Ports sim/battle.ts:2742-2749. Calls priorityChargeCallback directly;
    /// lifecycle owns action timing and active/fainted validation.
    /// PRNG: none directly; the callback may draw.
    pub fn run_priority_charge_move(&mut self, move_input: MoveInput, user: MonId) -> Relay {
        todo!("stage M: priority charge callback")
    }

    /// Ports sim/pokemon.ts:949-953. LockMove uses priorityEvent; true means no
    /// lock. A synthetic Recharge move remains distinct from a real dex move.
    /// PRNG: none directly; LockMove handler ordering/effects may draw.
    pub fn get_locked_move(&mut self, user: MonId) -> Option<MoveInput> {
        todo!("stage M: locked move query")
    }

    /// Ports sim/pokemon.ts:958-963. Preserves the maybeLocked visibility gate
    /// before dispatching SemiLockMove; used by choices/requests.
    /// PRNG: none directly; SemiLockMove handlers may draw.
    pub fn get_semi_locked_move(&mut self, user: MonId, restrict_data: bool) -> Option<MoveInput> {
        todo!("stage M: semi-locked move query")
    }

    /// Ports sim/pokemon.ts:904-915. Records lastMove, lastMoveUsed, targetLoc,
    /// and moveThisTurn. This is separate from PP consumption.
    /// PRNG: none.
    pub fn move_used(&mut self, user: MonId, move_handle: MoveHandle, target_loc: Option<i8>) {
        todo!("stage M: remember move use")
    }

    /// Ports charge plumbing in sim/battle-actions.ts:263-299,586-590 and
    /// data/conditions.ts:287-328 (twoturnmove). The condition handler owns
    /// duration and LockMove; this helper invokes PrepareHit and ChargeMove.
    /// PRNG: none directly; dispatched move/condition callbacks may draw.
    pub fn prepare_charge_move(
        &mut self,
        user: MonId,
        target: EventArg,
        move_handle: MoveHandle,
    ) -> Relay {
        todo!("stage M: charge move preparation")
    }

    /// Ports recharge integration from sim/pokemon.ts:970-976 and
    /// data/conditions.ts:364-380. No synthetic recharge dex entry is created;
    /// mustrecharge's callbacks own the cant line and volatile removal.
    /// PRNG: none directly; condition callback dispatch may draw.
    pub fn run_recharge(&mut self, user: MonId) -> Relay {
        todo!("stage M: synthetic recharge action")
    }

    /// Ports Struggle integration in sim/battle-actions.ts:226,281-290,1368-1387
    /// and data/moves.ts:18211-18227. Bypasses normal PP and uses direct recoil.
    /// PRNG: target selection and normal damage draw; no extra recoil draw.
    pub fn run_struggle(&mut self, user: MonId, target_loc: Option<i8>) -> Relay {
        todo!("stage M: Struggle execution")
    }
}
