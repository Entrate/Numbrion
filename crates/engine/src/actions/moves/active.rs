//! ActiveMove overlays and active-context plumbing. OWNER M.
#![allow(unused_variables)]
use crate::{
    actions::{MoveHandle, MoveInput},
    battle::Battle,
    event::{EffectRef, EventArg, Relay},
    ids::MonId,
    log::LogSink,
    state::scratch::ActiveMove,
};

impl<L: LogSink> Battle<L> {
    /// Ports sim/dex.ts:306-321. Dex input creates a mutable frame with hit=0;
    /// Active input returns that exact frame rather than cloning an existing copy.
    /// Recharge uses a synthetic move overlay and never a fake scoped dex ID.
    /// PRNG: none.
    pub fn get_active_move(&mut self, move_input: MoveInput) -> MoveHandle {
        todo!("stage M: materialize or reuse ActiveMove")
    }

    /// Ports the explicit clone idiom documented at sim/dex.ts:313-314 and
    /// mutable nested moves in sim/battle-actions.ts:377-442,1024-1028. Caller
    /// selects reset-to-dex or overlay-copy explicitly via the input variant.
    /// PRNG: none.
    pub fn copy_active_move(&mut self, move_handle: MoveHandle) -> MoveHandle {
        todo!("stage M: clone an ActiveMove overlay")
    }

    /// Ports mutable ActiveMove access in sim/battle-actions.ts:390-442.
    /// Scratch handles are scoped to a running action and cannot enter snapshots.
    /// PRNG: none.
    pub fn active_move(&self, move_handle: MoveHandle) -> &ActiveMove {
        self.scratch
            .moves
            .get(move_handle.0 as usize)
            .and_then(Option::as_ref)
            .expect("invalid or released active move")
    }

    /// Ports callback writes to ActiveMove in sim/battle-actions.ts:410-442.
    /// Drop this borrow before any event or cross-module battle call.
    /// PRNG: none.
    pub fn active_move_mut(&mut self, move_handle: MoveHandle) -> &mut ActiveMove {
        self.scratch
            .moves
            .get_mut(move_handle.0 as usize)
            .and_then(Option::as_mut)
            .expect("invalid or released active move")
    }

    /// Engine scratch reclamation for sim/dex.ts:316-321 allocations. A move
    /// remains live through nested events/calls that retain its handle; release
    /// only after those calls return. Does not clear the active battle context.
    /// PRNG: none.
    pub fn release_active_move(&mut self, move_handle: MoveHandle) {
        todo!("stage M: reclaim mutable move frame")
    }

    /// Ports sim/battle.ts:374-378. Assigns active move/user/target, with missing
    /// target falling back to the user; does not create an event frame.
    /// PRNG: none.
    pub fn set_active_move(
        &mut self,
        move_handle: Option<MoveHandle>,
        user: Option<MonId>,
        target: Option<MonId>,
    ) {
        self.scratch.active_move = move_handle.unwrap_or(MoveHandle::NONE);
        self.scratch.active_pokemon = user.unwrap_or(MonId::NONE);
        self.scratch.active_target = target.or(user).unwrap_or(MonId::NONE);
    }

    /// Ports sim/battle.ts:380-389. Stores successful lastMove then clears move,
    /// user, and target together; lastMove in Copy state retains its dex ID.
    /// PRNG: none.
    pub fn clear_active_move(&mut self, failed: bool) {
        if self.scratch.active_move != MoveHandle::NONE {
            if !failed {
                self.state.last_move = self.active_move(self.scratch.active_move).id;
            }
            self.scratch.active_move = MoveHandle::NONE;
            self.scratch.active_pokemon = MonId::NONE;
            self.scratch.active_target = MonId::NONE;
        }
    }

    /// Ports sim/battle-actions.ts:410-442. Executes ModifyTarget, the move's
    /// ModifyType/ModifyMove, then global ModifyType/ModifyMove in source order;
    /// retargets whenever the target kind changes, preserving callback sentinels.
    /// PRNG: retarget samples and dispatched event/callback draws only.
    pub fn modify_active_move(
        &mut self,
        move_handle: MoveHandle,
        user: MonId,
        target: EventArg,
        source_effect: EffectRef,
    ) -> (Relay, EventArg) {
        todo!("stage M: ModifyTarget/Type/Move pipeline")
    }

    /// Ports sim/battle.ts:370-372. Requires both move.hasSheerForce and the
    /// active user's unsuppressed Sheer Force before skipping after-secondary.
    /// PRNG: none.
    pub fn suppressing_secondaries(&self) -> bool {
        self.scratch.active_move != MoveHandle::NONE
            && self.active_move(self.scratch.active_move).runtime_flags
                & crate::state::scratch::move_runtime::HAS_SHEER_FORCE
                != 0
            && self.scratch.active_pokemon != MonId::NONE
            && self.has_ability(
                self.scratch.active_pokemon,
                &[crate::dex::ABILITY_SHEERFORCE],
            )
    }
}
