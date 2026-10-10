//! Outside/inside move callers. OWNER M.
use super::support::*;
use crate::{
    actions::{
        Attribution, HitEffect, HitOptions, MoveHandle, MoveInput, RunMoveOptions, Targets,
        UseMoveOptions,
    },
    battle::Battle,
    dex::{self, Category, MoveTarget},
    event::{EffectRef, EventArg, Relay, RunEventOptions},
    ids::{EffectId, EffectKind, EventId, MonId},
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::{ResultFlag, scratch::move_runtime as rt},
};

/// Item whose use clears `sourceEffect` in useMoveInner (battle-actions.ts:388); not in
/// the scoped Dex, so it never matches (`NONE`).
const CUSTAP_BERRY: EffectId = crate::effects::support::optional_id(dex::ITEMS_DATA, "custapberry");

/// Frames allocated by one scope; released when the scope ends.
#[derive(Default)]
struct Owned {
    handles: [Option<MoveHandle>; 4],
}
impl Owned {
    fn adopt(&mut self, h: MoveHandle) {
        let slot = self
            .handles
            .iter_mut()
            .find(|s| s.is_none())
            .expect("frame scope capacity");
        *slot = Some(h);
    }
}

/// The condition record that shares a move's key (`pokemon.addVolatile(move.id)`).
fn condition_of_move(move_id: EffectId) -> EffectId {
    dex::lookup(EffectKind::Condition, dex::effect(move_id).key)
        .expect("a cantusetwice move has a bare condition record")
}

impl<L: LogSink> Battle<L> {
    fn release_scope(&mut self, owned: Owned) {
        for h in owned.handles.into_iter().flatten() {
            self.release_active_move(h);
        }
    }

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
        let mut owned = Owned::default();
        self.run_move_body(
            move_input,
            user,
            target_loc,
            source_effect,
            options,
            &mut owned,
        );
        self.release_scope(owned);
        Relay::Undefined
    }

    fn run_move_body(
        &mut self,
        move_input: MoveInput,
        user: MonId,
        target_loc: Option<i8>,
        source_effect: EffectRef,
        options: RunMoveOptions,
        owned: &mut Owned,
    ) {
        let external = options.external;
        let mut source_effect = source_effect;
        self.state.pokemon[user.0 as usize].active_move_actions = self.state.pokemon
            [user.0 as usize]
            .active_move_actions
            .checked_add(1)
            .expect("activeMoveActions overflow");
        let mut target = self.get_target(user, move_input, target_loc, options.original_target);
        let mut base = self.get_active_move(move_input);
        if !matches!(move_input, MoveInput::Active(_)) {
            owned.adopt(base);
        }
        let priority = self.active_move(base).priority;
        let prankster_boosted = self.mflag(base, rt::PRANKSTER_BOOSTED);
        // (zMove and maxMove do not exist in this format.)
        if self.active_move(base).id != dex::MOVE_STRUGGLE && !external {
            let changed = self.run_event(
                EventId::OverrideAction,
                mon_arg(user),
                opt_mon_arg(target),
                EffectRef::ActiveMove(base.0),
                Relay::Undefined,
                RunEventOptions::default(),
            );
            if changed.truthy() && changed != Relay::Bool(true) {
                let Relay::Move(id) = changed else {
                    panic!("OverrideAction returned {changed:?}, not a move id");
                };
                base = self.get_active_move(MoveInput::Dex(id));
                owned.adopt(base);
                self.active_move_mut(base).priority = priority;
                if prankster_boosted {
                    self.set_mflag(base, rt::PRANKSTER_BOOSTED, true);
                }
                target = self.get_random_target(user, MoveInput::Active(base));
            }
        }
        let mv = base;
        self.set_mflag(mv, rt::EXTERNAL, external);

        self.set_active_move(Some(mv), Some(user), target);
        let effect = EffectRef::ActiveMove(mv.0);
        let will_try_move = self.run_event(
            EventId::BeforeMove,
            mon_arg(user),
            opt_mon_arg(target),
            effect,
            Relay::Undefined,
            RunEventOptions::default(),
        );
        if !will_try_move.truthy() {
            self.run_event(
                EventId::MoveAborted,
                mon_arg(user),
                opt_mon_arg(target),
                effect,
                Relay::Undefined,
                RunEventOptions::default(),
            );
            self.clear_active_move(true);
            // The event could have returned false or null: false counts as a failure for
            // Stomping Tantrum, null means the Pokemon had no option to choose anything.
            self.state.pokemon[user.0 as usize].move_this_turn_result = result_flag(will_try_move);
            return;
        }

        // Used exclusively for a hint later.
        let move_id = self.active_move(mv).id;
        let cant_use_twice = self.active_move(mv).flags & dex::FLAG_CANTUSETWICE != 0;
        if cant_use_twice && self.state.pokemon[user.0 as usize].last_move == move_id {
            self.add_volatile(user, condition_of_move(move_id), Attribution::NONE, None);
        }

        // `move.beforeMoveCallback`: no scoped move defines one (Focus Punch and Shell Trap
        // are not in the generated Dex).
        if !external {
            if self.get_locked_move(user).is_none() {
                let deducted = self.deduct_pp(user, self.active_move(base).id, None);
                if deducted == 0.0 && self.active_move(mv).id != dex::MOVE_STRUGGLE {
                    self.add(LogEntry::new(
                        "cant",
                        &[
                            LogArg::Mon(user),
                            LogArg::Text("nopp"),
                            LogArg::Effect(effect),
                        ],
                        &[],
                    ));
                    self.clear_active_move(true);
                    self.state.pokemon[user.0 as usize].move_this_turn_result = ResultFlag::False;
                    return;
                }
            } else {
                source_effect = EffectRef::Dex(dex::CONDITION_LOCKEDMOVE);
            }
            self.move_used(user, mv, target_loc);
        }

        // Dancer Petal Dance hack.
        let no_lock = external && self.get_volatile(user, dex::CONDITION_LOCKEDMOVE).is_none();

        let move_did_something = self.use_move(
            MoveInput::Active(base),
            user,
            opt_mon_arg(target),
            source_effect,
            UseMoveOptions::default(),
        );
        let current = self.scratch.active_move;
        self.state.last_successful_move =
            if move_did_something.truthy() && current != MoveHandle::NONE {
                self.active_move(current).id
            } else {
                EffectId::NONE
            };
        let mut mv = mv;
        if current != MoveHandle::NONE {
            mv = current;
            // An inside caller can leave its completed frame globally active (Sleep
            // Talk/Magic Bounce). The outside caller retains that object through
            // AfterMove and Dancer, even if a callback clears the globals meanwhile.
            if self.mflag(mv, PENDING_RELEASE) {
                self.set_mflag(mv, PENDING_RELEASE, false);
                owned.adopt(mv);
            }
        }
        let effect = EffectRef::ActiveMove(mv.0);
        self.single_event(
            EventId::AfterMove,
            effect,
            None,
            mon_arg(user),
            opt_mon_arg(target),
            effect,
            Relay::Undefined,
            None,
        );
        self.run_event(
            EventId::AfterMove,
            mon_arg(user),
            opt_mon_arg(target),
            effect,
            Relay::Undefined,
            RunEventOptions::default(),
        );
        let move_id = self.active_move(mv).id;
        let cant_use_twice = self.active_move(mv).flags & dex::FLAG_CANTUSETWICE != 0;
        if cant_use_twice && self.remove_volatile(user, condition_of_move(move_id)) {
            self.add(LogEntry::new(
                "-hint",
                &[LogArg::Parts(&[
                    LogArg::Text("Some effects can force a Pokemon to use "),
                    LogArg::Effect(effect),
                    LogArg::Text(" again in a row."),
                ])],
                &[],
            ));
        }

        // Dancer's activation order is completely different from any other event, so it is
        // handled separately.
        if self.active_move(mv).flags & dex::FLAG_DANCE != 0
            && move_did_something.truthy()
            && !self.mflag(mv, rt::EXTERNAL)
        {
            let first_target = self.scratch.active_target;
            self.run_dancers(user, mv, first_target);
        }
        if no_lock {
            self.delete_volatile_raw(user, dex::CONDITION_LOCKEDMOVE);
        }
        self.faint_messages(false, false, true);
        self.check_win(None);
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
        self.state.pokemon[user.0 as usize].move_this_turn_result = ResultFlag::Undefined;
        let result = self.use_move_inner(move_input, user, target, source_effect, options);
        // `oldMoveResult` is undefined: only an untouched result is replaced.
        if self.state.pokemon[user.0 as usize].move_this_turn_result == ResultFlag::Undefined {
            self.state.pokemon[user.0 as usize].move_this_turn_result = result_flag(result);
        }
        result
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
        let mut owned = Owned::default();
        let result =
            self.use_move_inner_body(move_input, user, target, source_effect, options, &mut owned);
        self.release_scope(owned);
        result
    }

    fn use_move_inner_body(
        &mut self,
        move_input: MoveInput,
        user: MonId,
        target: EventArg,
        source_effect: EffectRef,
        _options: UseMoveOptions,
        owned: &mut Owned,
    ) -> Relay {
        let mut source_effect = source_effect;
        // The calling effect, if any.
        if source_effect == EffectRef::None
            && self.event_effect_id(self.scratch.current_effect) != EffectId::NONE
        {
            source_effect = self.scratch.current_effect;
        }
        if source_effect != EffectRef::None {
            let id = self.event_effect_id(source_effect);
            if id != EffectId::NONE && (id == dex::MOVE_INSTRUCT || id == CUSTAP_BERRY) {
                source_effect = EffectRef::None;
            }
        }
        let mv = self.get_active_move(move_input);
        if !matches!(move_input, MoveInput::Active(_)) {
            owned.adopt(mv);
        }
        // (`pokemon.lastMoveUsed` is not part of the Copy state; Z and Max moves do not exist.)
        let (result, target) = self.modify_active_move(mv, user, target, source_effect);
        if !result.truthy() || self.mon_fainted(user) {
            return Relay::FAIL;
        }
        let mv = match result {
            Relay::ActiveMove(h) => MoveHandle(h),
            other => panic!("ModifyMove returned {other:?} instead of a move"),
        };
        let effect = EffectRef::ActiveMove(mv.0);

        // addMove('move', pokemon, movename, `${target}${attrs}`)
        let target_log = match arg_to_mon(target) {
            Some(m) => LogArg::Mon(m),
            None if matches!(target, EventArg::Undefined) => LogArg::Text("undefined"),
            None => LogArg::Text("null"),
        };
        if source_effect != EffectRef::None {
            self.add_move(LogEntry::new(
                "move",
                &[LogArg::Mon(user), LogArg::Effect(effect), target_log],
                &[LogTag::From(source_effect)],
            ));
        } else {
            self.add_move(LogEntry::new(
                "move",
                &[LogArg::Mon(user), LogArg::Effect(effect), target_log],
                &[],
            ));
        }

        let Some(target) = arg_to_mon(target) else {
            self.attr_last_move(MoveLineEdit::NoTarget);
            self.add(LogEntry::new("-fail", &[LogArg::Mon(user)], &[]));
            return Relay::FAIL;
        };

        let move_targets = self.get_move_targets(user, mv, target);
        let targets = move_targets.hit;
        let mut target = target;
        if targets.count() > 0 {
            target = targets.mon_at(targets.count() - 1).expect("hit target"); // in case of redirection
        }

        let caller_move_for_pressure = self.source_move_with_pp(source_effect);
        if source_effect == EffectRef::None || caller_move_for_pressure.is_some() {
            let mut extra_pp = 0.0;
            let pressure = move_targets.pressure;
            for i in 0..pressure.count() {
                let source = pressure.mon_at(i).expect("pressure target");
                let pp_drop = self.run_event(
                    EventId::DeductPP,
                    mon_arg(source),
                    mon_arg(user),
                    effect,
                    Relay::Undefined,
                    RunEventOptions::default(),
                );
                if pp_drop != Relay::Bool(true) {
                    if let (true, Relay::Number(n)) = (pp_drop.truthy(), pp_drop) {
                        extra_pp += n;
                    }
                }
            }
            if extra_pp > 0.0 {
                let id = caller_move_for_pressure.unwrap_or_else(|| self.active_move(mv).id);
                self.deduct_pp(user, id, Some(extra_pp));
            }
        }

        let mut try_move_result = self.single_event(
            EventId::TryMove,
            effect,
            None,
            mon_arg(user),
            mon_arg(target),
            effect,
            Relay::Undefined,
            None,
        );
        if try_move_result.truthy() {
            try_move_result = self.run_event(
                EventId::TryMove,
                mon_arg(user),
                mon_arg(target),
                effect,
                Relay::Undefined,
                RunEventOptions::default(),
            );
        }
        if !try_move_result.truthy() {
            return try_move_result;
        }

        self.single_event(
            EventId::UseMoveMessage,
            effect,
            None,
            mon_arg(user),
            mon_arg(target),
            effect,
            Relay::Undefined,
            None,
        );

        if !self.mflag(mv, rt::IGNORE_IMMUNITY_PRESENT) {
            let status = self.active_move(mv).category == Category::Status;
            self.set_mflag(mv, rt::IGNORE_IMMUNITY_PRESENT, true);
            self.set_mflag(mv, rt::IGNORE_IMMUNITY, status);
        }

        if self.active_move(mv).self_destruct == dex::SelfDestruct::Always {
            self.faint(user, Attribution::from_move(user, effect));
        }

        let mut move_result = false;
        let kind = self.active_move(mv).target;
        if matches!(
            kind,
            MoveTarget::All | MoveTarget::FoeSide | MoveTarget::AllySide
        ) {
            let damage = self.try_move_hit(targets, user, mv);
            if damage == Relay::NotFail {
                self.state.pokemon[user.0 as usize].move_this_turn_result = ResultFlag::Null;
            }
            if damage.truthy() || damage == Relay::Number(0.0) || damage == Relay::Undefined {
                move_result = true;
            }
        } else {
            if targets.count() == 0 {
                self.attr_last_move(MoveLineEdit::NoTarget);
                self.add(LogEntry::new("-fail", &[LogArg::Mon(user)], &[]));
                return Relay::FAIL;
            }
            move_result = self.try_spread_move_hit(targets, user, mv, false).truthy();
        }
        if self.active_move(mv).self_boosts.len > 0 && move_result {
            self.move_hit(
                Targets::from_mons(&[user]),
                user,
                mv,
                HitEffect::SelfBoost,
                HitOptions {
                    secondary: false,
                    self_hit: true,
                },
            );
        }
        if self.mon_hp(user) == 0 {
            self.faint(user, Attribution::from_move(user, effect));
        }

        if !move_result {
            let original_hp = self.mon_hp(user);
            self.single_event(
                EventId::MoveFail,
                effect,
                None,
                mon_arg(target),
                mon_arg(user),
                effect,
                Relay::Undefined,
                None,
            );
            if user != target && self.active_move(mv).category != Category::Status {
                self.run_event(
                    EventId::EmergencyExit,
                    mon_arg(user),
                    mon_arg(user),
                    EffectRef::None,
                    Relay::Number(f64::from(original_hp)),
                    RunEventOptions::default(),
                );
            }
            return Relay::FAIL;
        }

        // 'futuremove' is not a scoped flag.
        if !self.suppressing_secondaries() {
            let original_hp = self.mon_hp(user);
            self.single_event(
                EventId::AfterMoveSecondarySelf,
                effect,
                None,
                mon_arg(user),
                mon_arg(target),
                effect,
                Relay::Undefined,
                None,
            );
            self.run_event(
                EventId::AfterMoveSecondarySelf,
                mon_arg(user),
                mon_arg(target),
                effect,
                Relay::Undefined,
                RunEventOptions::default(),
            );
            if user != target && self.active_move(mv).category != Category::Status {
                self.run_event(
                    EventId::EmergencyExit,
                    mon_arg(user),
                    mon_arg(user),
                    EffectRef::None,
                    Relay::Number(f64::from(original_hp)),
                    RunEventOptions::default(),
                );
            }
        }

        Relay::Bool(true)
    }

    /// `(sourceEffect as ActiveMove).pp ? sourceEffect : null`: the id of a calling Move.
    fn source_move_with_pp(&self, effect: EffectRef) -> Option<EffectId> {
        match effect {
            EffectRef::ActiveMove(i) => {
                let id = self.active_move(MoveHandle(i)).id;
                (id != EffectId::NONE).then_some(id)
            }
            EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => Some(id),
            _ => None,
        }
    }

    /// Ports the Dancer block in sim/battle-actions.ts:315-345. Uses raw stored
    /// speed and reverse ability effectOrder; retains the original dance target.
    /// PRNG: none for Dancer ordering; each recursive run_move may draw.
    pub fn run_dancers(&mut self, user: MonId, move_handle: MoveHandle, first_target: MonId) {
        let active = self.get_all_active(false);
        let mut dancers = [MonId::NONE; 4];
        let mut len = 0;
        for &current in active.as_slice() {
            if current == user {
                continue;
            }
            if self.has_ability(current, &[dex::ABILITY_DANCER])
                && !self.is_semi_invulnerable(current)
            {
                dancers[len] = current;
                len += 1;
            }
        }
        // Dancer activates in order of lowest speed stat to highest. The speed stat used is
        // after volatile replacements like Speed Swap, but before multipliers like Agility.
        // Ties go to whichever Pokemon has had the ability for the least amount of time.
        // (Array.prototype.sort is stable.)
        let key = |b: &Self, m: MonId| -> (i32, i64) {
            let p = &b.state.pokemon[m.0 as usize];
            (
                i32::from(p.stored_stats[4]),
                i64::from(b.state.effects.cells[p.ability_state.0 as usize].effect_order),
            )
        };
        for i in 1..len {
            let mut j = i;
            while j > 0 {
                let (a, b) = (key(self, dancers[j - 1]), key(self, dancers[j]));
                // comparator(a, b) = -(b.spe - a.spe) || b.effectOrder - a.effectOrder
                let cmp = if a.0 != b.0 {
                    i64::from(a.0 - b.0)
                } else {
                    b.1 - a.1
                };
                // `dancers[j - 1]` stays first unless it compares greater than `dancers[j]`.
                if cmp > 0 {
                    dancers.swap(j - 1, j);
                    j -= 1;
                } else {
                    break;
                }
            }
        }
        let move_id = self.active_move(move_handle).id;
        for &dancer in &dancers[..len] {
            if self.faint_messages(false, false, true).truthy() {
                break;
            }
            if self.mon_fainted(dancer) {
                continue;
            }
            self.add(LogEntry::new(
                "-activate",
                &[LogArg::Mon(dancer), LogArg::Text("ability: Dancer")],
                &[],
            ));
            let dancers_target =
                if !self.is_ally(first_target, Some(dancer)) && self.is_ally(user, Some(dancer)) {
                    first_target
                } else {
                    user
                };
            let loc = self.get_loc_of(dancer, dancers_target);
            self.run_move(
                MoveInput::Dex(move_id),
                dancer,
                Some(loc),
                EffectRef::Dex(dex::ABILITY_DANCER),
                RunMoveOptions {
                    external: true,
                    original_target: None,
                },
            );
        }
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
        let target = self.get_target(user, move_input, target_loc, None);
        if target.is_none() {
            return Relay::FAIL;
        }
        // No scoped move defines `beforeTurnCallback`; the source throws for such an action.
        panic!("beforeTurnMove has no beforeTurnCallback");
    }

    /// Ports sim/battle.ts:2742-2749. Calls priorityChargeCallback directly;
    /// lifecycle owns action timing and active/fainted validation.
    /// PRNG: none directly; the callback may draw.
    pub fn run_priority_charge_move(&mut self, move_input: MoveInput, user: MonId) -> Relay {
        let _ = (move_input, user);
        // No scoped move defines `priorityChargeCallback`; the source throws for such an action.
        panic!("priorityChargeMove has no priorityChargeCallback");
    }

    /// Ports sim/pokemon.ts:949-953. LockMove uses priorityEvent; true means no
    /// lock. A synthetic Recharge move remains distinct from a real dex move.
    /// PRNG: none directly; LockMove handler ordering/effects may draw.
    pub fn get_locked_move(&mut self, user: MonId) -> Option<MoveInput> {
        let locked = self.priority_event(
            EventId::LockMove,
            mon_arg(user),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            false,
        );
        Self::relay_to_locked_move(locked)
    }

    fn relay_to_locked_move(locked: Relay) -> Option<MoveInput> {
        match locked {
            // `lockedMove === true ? null : lockedMove`; falsy values mean "not locked".
            Relay::Bool(true) => None,
            r if !r.truthy() => None,
            Relay::Move(id) => Some(MoveInput::Dex(id)),
            Relay::PseudoMove(crate::event::SyntheticEffect::Recharge) => Some(MoveInput::Recharge),
            other => panic!("LockMove returned {other:?}, not a move"),
        }
    }

    /// Ports sim/pokemon.ts:958-963. Preserves the maybeLocked visibility gate
    /// before dispatching SemiLockMove; used by choices/requests.
    /// PRNG: none directly; SemiLockMove handlers may draw.
    pub fn get_semi_locked_move(&mut self, user: MonId, restrict_data: bool) -> Option<MoveInput> {
        if restrict_data
            && self.state.pokemon[user.0 as usize].flags & crate::state::mon_flags::MAYBE_LOCKED
                != 0
        {
            return None;
        }
        let locked = self.priority_event(
            EventId::SemiLockMove,
            mon_arg(user),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            false,
        );
        Self::relay_to_locked_move(locked)
    }

    /// Ports sim/pokemon.ts:904-915. Records lastMove, lastMoveUsed, targetLoc,
    /// and moveThisTurn. This is separate from PP consumption.
    /// PRNG: none.
    pub fn move_used(&mut self, user: MonId, move_handle: MoveHandle, target_loc: Option<i8>) {
        let id = self.active_move(move_handle).id;
        let p = &mut self.state.pokemon[user.0 as usize];
        p.last_move = id;
        // lastMoveTargetLoc may be undefined; every reader tests it for falsiness.
        p.last_move_target_loc = target_loc.unwrap_or(0);
    }

    /// Ports charge plumbing in sim/battle-actions.ts:263-299,586-590 and
    /// data/conditions.ts:287-328 (twoturnmove). The condition handler owns
    /// duration and LockMove; this helper runs the ChargeMove event that a charge
    /// move's TryMove handler consults before adding twoturnmove (Power Herb).
    /// The condition's own PrepareHit run stays in its Start handler, where the source
    /// fires it after addVolatile.
    /// PRNG: none directly; dispatched move/condition callbacks may draw.
    pub fn prepare_charge_move(
        &mut self,
        user: MonId,
        target: EventArg,
        move_handle: MoveHandle,
    ) -> Relay {
        self.run_event(
            EventId::ChargeMove,
            mon_arg(user),
            target,
            EffectRef::ActiveMove(move_handle.0),
            Relay::Undefined,
            RunEventOptions::default(),
        )
    }

    /// Ports recharge integration from sim/pokemon.ts:970-976 and
    /// data/conditions.ts:364-380. No synthetic recharge dex entry is created;
    /// mustrecharge's callbacks own the cant line and volatile removal.
    /// PRNG: none directly; condition callback dispatch may draw.
    pub fn run_recharge(&mut self, user: MonId) -> Relay {
        // Side choice: `volatiles[lockedMove]?.targetLoc || lastMoveTargetLoc || 0`; the
        // 'recharge' pseudo move has no volatile.
        let loc = self.state.pokemon[user.0 as usize].last_move_target_loc;
        self.run_move(
            MoveInput::Recharge,
            user,
            Some(loc),
            EffectRef::None,
            RunMoveOptions::default(),
        )
    }

    /// Ports Struggle integration in sim/battle-actions.ts:226,281-290,1368-1387 and
    /// data/moves.ts:18211-18227. Bypasses normal PP and uses direct recoil.
    /// PRNG: target selection and normal damage draw; no extra recoil draw.
    pub fn run_struggle(&mut self, user: MonId, target_loc: Option<i8>) -> Relay {
        self.run_move(
            MoveInput::Dex(dex::MOVE_STRUGGLE),
            user,
            target_loc,
            EffectRef::None,
            RunMoveOptions::default(),
        )
    }
}
