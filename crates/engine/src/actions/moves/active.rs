//! ActiveMove overlays and active-context plumbing. OWNER M.
use super::support::*;
use crate::{
    actions::{MoveHandle, MoveInput},
    battle::Battle,
    dex::{self, MoveTarget},
    event::{EffectRef, EventArg, Relay, RunEventOptions},
    ids::{EffectKind, EventId, MonId},
    log::LogSink,
    state::scratch::{ActiveMove, move_runtime as rt},
};

impl<L: LogSink> Battle<L> {
    /// Reserve a scratch slot. A frame whose creating scope ended while it was still the
    /// global active move is reclaimed here once it no longer is (lazy reclamation keeps
    /// `clear_active_move` free to read the frame it replaces, as battle.ts:380-388 does).
    fn alloc_move_frame(&mut self, mv: ActiveMove) -> MoveHandle {
        let slot = match self.scratch.moves.iter().position(Option::is_none) {
            Some(i) => i,
            None => (0..self.scratch.moves.len())
                .find(|&i| {
                    self.scratch.moves[i]
                        .as_ref()
                        .is_some_and(|m| m.runtime_flags & PENDING_RELEASE != 0)
                        && self.scratch.active_move != MoveHandle(i as u8)
                })
                .expect("active move frame capacity exceeded"),
        };
        self.scratch.moves[slot] = Some(mv);
        MoveHandle(slot as u8)
    }

    /// Ports sim/dex.ts:306-321. Dex input creates a mutable frame with hit=0;
    /// Active input returns that exact frame rather than cloning an existing copy.
    /// Recharge uses a synthetic move overlay and never a fake scoped dex ID.
    /// PRNG: none.
    pub fn get_active_move(&mut self, move_input: MoveInput) -> MoveHandle {
        match move_input {
            MoveInput::Active(h) => {
                // `typeof move.hit === 'number'` => returned unchanged.
                assert!(
                    self.scratch
                        .moves
                        .get(h.0 as usize)
                        .is_some_and(Option::is_some),
                    "invalid or released active move"
                );
                h
            }
            MoveInput::Dex(id) => self.alloc_move_frame(build_active_move(id)),
            MoveInput::Recharge => self.alloc_move_frame(build_recharge_move()),
        }
    }

    /// Ports the explicit clone idiom documented at sim/dex.ts:313-314 and
    /// mutable nested moves in sim/battle-actions.ts:377-442,1024-1028. Caller
    /// selects reset-to-dex or overlay-copy explicitly via the input variant.
    /// PRNG: none.
    pub fn copy_active_move(&mut self, move_handle: MoveHandle) -> MoveHandle {
        let mut copy = *self.active_move(move_handle);
        copy.runtime_flags &= !PENDING_RELEASE;
        self.alloc_move_frame(copy)
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
    /// A frame that is still the global `battle.activeMove` is only marked; it is
    /// reclaimed when a later allocation finds it no longer referenced.
    /// PRNG: none.
    pub fn release_active_move(&mut self, move_handle: MoveHandle) {
        if move_handle == MoveHandle::NONE {
            return;
        }
        let i = move_handle.0 as usize;
        if self.scratch.moves[i].is_none() {
            return;
        }
        if self.scratch.active_move == move_handle {
            self.active_move_mut(move_handle).runtime_flags |= PENDING_RELEASE;
        } else {
            self.scratch.moves[i] = None;
        }
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
        // Scope-completed moves no longer have a global reference. Reclaim them here
        // so decision boundaries contain no abandoned move frames; still-owned outer
        // moves retain their slots until their own creating scope ends.
        for slot in &mut self.scratch.moves {
            if slot
                .as_ref()
                .is_some_and(|m| m.runtime_flags & PENDING_RELEASE != 0)
            {
                *slot = None;
            }
        }
    }

    /// Ports sim/battle-actions.ts:410-442 (the body of useMoveInner between the
    /// priority inheritance and the `!move || pokemon.fainted` test). Executes
    /// ModifyTarget, the move's ModifyType/ModifyMove, then global ModifyType/ModifyMove in
    /// source order; retargets whenever the target kind changes, preserving callback
    /// sentinels. Returns the final `move` relay (falsy aborts the move) and `target`.
    /// `target` is undefined, null or a Pokemon (retaining defaulting order).
    /// PRNG: retarget samples and dispatched event/callback draws only.
    pub fn modify_active_move(
        &mut self,
        move_handle: MoveHandle,
        user: MonId,
        target: EventArg,
        source_effect: EffectRef,
    ) -> (Relay, EventArg) {
        let mut target = target;
        let mut h = move_handle;
        // 410-413: inherit the caller's priority and Prankster boost.
        let current = self.scratch.active_move;
        if current != MoveHandle::NONE {
            let (priority, prankster) = {
                let a = self.active_move(current);
                (a.priority, a.runtime_flags & rt::PRANKSTER_BOOSTED != 0)
            };
            self.active_move_mut(h).priority = priority;
            if !self.mflag(h, rt::HAS_BOUNCED) {
                self.set_mflag(h, rt::PRANKSTER_BOOSTED, prankster);
            }
        }
        let base_target = self.active_move(h).target;
        // 414-417: ModifyTarget with `{target}` as relay and onEffect. No scoped effect
        // listens; a handler would hand back the replacement Pokemon.
        let relay = match arg_to_mon(target) {
            Some(m) => Relay::Pokemon(m),
            None => Relay::Null,
        };
        let out = self.run_event(
            EventId::ModifyTarget,
            mon_arg(user),
            target,
            EffectRef::ActiveMove(h.0),
            relay,
            RunEventOptions {
                on_effect: true,
                fast_exit: false,
            },
        );
        if let Relay::Pokemon(m) = out {
            target = mon_arg(m);
        }
        // 418-421
        if matches!(target, EventArg::Undefined) {
            target = opt_mon_arg(self.get_random_target(user, MoveInput::Active(h)));
        }
        if matches!(
            self.active_move(h).target,
            MoveTarget::SelfTarget | MoveTarget::Allies
        ) {
            target = mon_arg(user);
        }
        // 422-425: sourceEffect.id and its ignoreAbility property overwrite the move's.
        if source_effect != EffectRef::None {
            let ignore = self.effect_ignore_ability(source_effect);
            let m = self.active_move_mut(h);
            m.source_effect = source_effect;
            if ignore {
                m.runtime_flags |= rt::IGNORE_ABILITY;
            } else {
                m.runtime_flags &= !rt::IGNORE_ABILITY;
            }
        }
        // 428
        self.set_active_move(Some(h), Some(user), arg_to_mon(target));
        // 430-437
        let ev = EffectRef::ActiveMove(h.0);
        for event in [EventId::ModifyType, EventId::ModifyMove] {
            self.single_event(
                event,
                ev,
                None,
                mon_arg(user),
                target,
                ev,
                Relay::ActiveMove(h.0),
                None,
            );
        }
        if base_target != self.active_move(h).target {
            // Target changed in ModifyMove, so we must adjust it here.
            target = opt_mon_arg(self.get_random_target(user, MoveInput::Active(h)));
        }
        // 438-443
        let mut result = Relay::ActiveMove(h.0);
        for event in [EventId::ModifyType, EventId::ModifyMove] {
            result = self.run_event(
                event,
                mon_arg(user),
                target,
                EffectRef::ActiveMove(h.0),
                result,
                RunEventOptions::default(),
            );
            if let Relay::ActiveMove(replacement) = result {
                h = MoveHandle(replacement);
            }
            if !result.truthy() {
                // `move = false`: later statements only test `!move`.
                break;
            }
        }
        if result.truthy() && base_target != self.active_move(h).target {
            // Adjust again (compares with the ORIGINAL baseTarget).
            target = opt_mon_arg(self.get_random_target(user, MoveInput::Active(h)));
        }
        (result, target)
    }

    /// `sourceEffect.ignoreAbility` for the effect kinds that can call a move.
    fn effect_ignore_ability(&self, effect: EffectRef) -> bool {
        match effect {
            EffectRef::ActiveMove(i) => {
                self.active_move(MoveHandle(i)).runtime_flags & rt::IGNORE_ABILITY != 0
            }
            EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
                dex::move_data(id).traits & dex::MOVE_TRAIT_IGNOREABILITY != 0
            }
            _ => false,
        }
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
