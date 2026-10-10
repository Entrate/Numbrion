//! Ordered hit stages and multihit orchestration. OWNER M.
use super::support::*;
use crate::{
    actions::{
        Attribution, HitEffect, HitOptions, HitTarget, ImmunityMessage, ImmunitySource, MoveHandle,
        MoveInput, TargetResults, Targets,
    },
    battle::Battle,
    dex::{self, ImmunityId, MoveTarget},
    event::{EffectRef, EventArg, EventTargets, Relay, RunEventOptions},
    ids::{EventId, MonId, TypeId},
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    math,
    state::{
        Status,
        scratch::{MoveAccuracy, OrderedBoosts, move_runtime as rt},
    },
};

/// 35-35-15-15 out of 100 for 2-3-4-5 hits (battle-actions.ts:863).
const MULTIHIT_TABLE: [u8; 20] = [2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 5, 5, 5];
/// `boostTable` of the repeated multiaccuracy roll (battle-actions.ts:907).
const BOOST_TABLE: [f64; 7] = [1.0, 4.0 / 3.0, 5.0 / 3.0, 2.0, 7.0 / 3.0, 8.0 / 3.0, 3.0];
const POISON: TypeId = TypeId(4);
const ACCURACY_STAGE: usize = 5;
const EVASION_STAGE: usize = 6;

/// JS number coercion of a relay used as an arithmetic operand.
fn relay_f64(r: Relay) -> f64 {
    match r {
        Relay::Number(n) => n,
        Relay::Bool(true) => 1.0,
        Relay::Bool(false) | Relay::Null | Relay::NotFail => 0.0,
        Relay::Undefined => f64::NAN,
        other => panic!("accuracy relay {other:?} is not numeric"),
    }
}

impl<L: LogSink> Battle<L> {
    /// `Dex.getImmunity(type, pokemon)`: the pokemon's current types against a
    /// non-damage immunity row. Reads types through the Type event like getTypes().
    fn dex_immunity(&mut self, pokemon: MonId, id: ImmunityId) -> bool {
        let types = self.get_types(pokemon, false, false);
        types.values[..types.len as usize]
            .iter()
            .all(|t| dex::IMMUNITY_CHART[(t.0 - 1) as usize][id as usize] != 3)
    }

    /// `Dex.getImmunity(move.status, pokemon)`; statuses without a chart row never grant it.
    fn dex_status_immunity(&mut self, pokemon: MonId, status: crate::ids::EffectId) -> bool {
        let id = match status {
            dex::CONDITION_BRN => ImmunityId::Brn,
            dex::CONDITION_FRZ => ImmunityId::Frz,
            dex::CONDITION_PAR => ImmunityId::Par,
            dex::CONDITION_PSN => ImmunityId::Psn,
            dex::CONDITION_TOX => ImmunityId::Tox,
            _ => return true,
        };
        self.dex_immunity(pokemon, id)
    }

    /// `runEvent('ModifyBoost', pokemon, null, null, {...pokemon.boosts})`: all seven stages
    /// handed to the event as a fresh object; the (possibly replaced) object is copied back.
    fn modified_boosts(&mut self, pokemon: MonId) -> OrderedBoosts {
        let boosts = OrderedBoosts {
            values: self.state.pokemon[pokemon.0 as usize].boosts,
            order: [0, 1, 2, 3, 4, 5, 6],
            len: 7,
            present: 0x7f,
        };
        let original = self.stash_boosts(boosts);
        let result = self.run_event(
            EventId::ModifyBoost,
            mon_arg(pokemon),
            EventArg::Null,
            EffectRef::None,
            Relay::Boosts(original),
            RunEventOptions::default(),
        );
        let Relay::Boosts(handle) = result else {
            panic!("ModifyBoost returned a non-object relay");
        };
        let modified = *self.scratch_boosts(handle);
        self.release_relay(result);
        if handle != original {
            self.release_relay(Relay::Boosts(original));
        }
        modified
    }

    /// `clampIntRange(boosts[stat], -6, 6)`; a deleted key is `undefined`, i.e. zero.
    fn boost_stage(boosts: &OrderedBoosts, index: usize) -> f64 {
        if boosts.present & (1 << index) == 0 {
            0.0
        } else {
            clamp_int_range(f64::from(boosts.values[index]), Some(-6.0), Some(6.0))
        }
    }

    /// `boosts[stat]` as an arithmetic operand: a deleted key is `undefined`, i.e. NaN.
    fn raw_stage(boosts: &OrderedBoosts, index: usize) -> f64 {
        if boosts.present & (1 << index) == 0 {
            f64::NAN
        } else {
            f64::from(boosts.values[index])
        }
    }

    /// The move's own callback (or a secondary's) for a hit event, as singleEvent with the
    /// move as effect (battle-actions.ts:1030-1035, 1257-1268, 1116). A missing callback is
    /// the implicit `true`.
    fn hit_single_event(
        &mut self,
        view: &HitView,
        event: EventId,
        move_handle: MoveHandle,
        target: EventArg,
        source: MonId,
    ) -> Relay {
        match self.hit_hook(view, event) {
            Some(hook) => {
                let effect = EffectRef::ActiveMove(move_handle.0);
                self.single_event(
                    event,
                    effect,
                    None,
                    target,
                    mon_arg(source),
                    effect,
                    Relay::Undefined,
                    Some(hook),
                )
            }
            None => Relay::Bool(true),
        }
    }

    /// `Try && PrepareHit && PrepareHit` of trySpreadMoveHit/tryMoveHit: JS `&&` yields
    /// the first falsy value, otherwise the last.
    fn try_prepare_hit(
        &mut self,
        move_handle: MoveHandle,
        user: MonId,
        target: Option<MonId>,
    ) -> Relay {
        let effect = EffectRef::ActiveMove(move_handle.0);
        let mut r = self.single_event(
            EventId::Try,
            effect,
            None,
            mon_arg(user),
            opt_mon_arg_undef(target),
            effect,
            Relay::Undefined,
            None,
        );
        if r.truthy() {
            r = self.single_event(
                EventId::PrepareHit,
                effect,
                None,
                opt_mon_arg_undef(target),
                mon_arg(user),
                effect,
                Relay::Undefined,
                None,
            );
        }
        if r.truthy() {
            r = self.run_event(
                EventId::PrepareHit,
                mon_arg(user),
                opt_mon_arg_undef(target),
                effect,
                Relay::Undefined,
                RunEventOptions::default(),
            );
        }
        r
    }

    fn log_fail(&mut self, user: MonId) {
        self.add(LogEntry::new("-fail", &[LogArg::Mon(user)], &[]));
    }

    /// Ports sim/battle-actions.ts:546-616. Calls Try/PrepareHit, runs the eight
    /// Gen 9 hit stages, filters in original target order, and edits spread log.
    /// PRNG: target-stage draws plus dispatched callbacks, in exact stage order.
    pub fn try_spread_move_hit(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
        not_active: bool,
    ) -> Relay {
        let mut targets = targets;
        if targets.count() > 1 && !self.input_smart_target(MoveInput::Active(move_handle)) {
            self.set_mflag(move_handle, rt::SPREAD_HIT, true);
        }
        if not_active {
            self.set_active_move(Some(move_handle), Some(user), targets.mon_at(0));
        }
        let first = targets.mon_at(0);
        let hit_result = self.try_prepare_hit(move_handle, user, first);
        if !hit_result.truthy() {
            if is_false(hit_result) {
                self.log_fail(user);
                self.attr_last_move(MoveLineEdit::Still);
            }
            return Relay::Bool(hit_result == Relay::NotFail);
        }

        let mut at_least_one_failure = false;
        for step in 0..8 {
            let results = match step {
                // 0. check for semi invulnerability
                0 => self.hit_step_invulnerability_event(targets, user, move_handle),
                // 1. the 'TryHit' event (Protect, Magic Bounce, Volt Absorb, etc.)
                1 => self.hit_step_try_hit_event(targets, user, move_handle),
                // 2. type immunity (step 1 in gens 4-6)
                2 => self.hit_step_type_immunity(targets, user, move_handle),
                // 3. various move-specific immunities
                3 => self.hit_step_try_immunity(targets, user, move_handle),
                // 4. accuracy
                4 => self.hit_step_accuracy(targets, user, move_handle),
                // 5. break protection effects
                5 => self.hit_step_break_protect(targets, user, move_handle),
                // 6. steal positive boosts (Spectral Thief)
                6 => self.hit_step_steal_boosts(targets, user, move_handle),
                // 7. the loop that processes each hit of the move
                _ => self.hit_step_move_hit_loop(targets, user, move_handle),
            };
            let Some(results) = results else {
                continue;
            };
            let mut survivors = Targets::default();
            for i in 0..targets.count() {
                if keeps(results.at(i)) {
                    survivors.push_target(targets.at(i));
                }
            }
            targets = survivors;
            at_least_one_failure = at_least_one_failure || results.some_false();
            if at_least_one_failure && self.input_smart_target(MoveInput::Active(move_handle)) {
                self.disable_smart_target(move_handle);
            }
            if targets.count() == 0 {
                break;
            }
        }

        {
            let m = self.active_move_mut(move_handle);
            m.hit_targets = [MonId::NONE; 4];
            for i in 0..targets.count() {
                m.hit_targets[i] = targets.mon_at(i).expect("hit target");
            }
            m.hit_target_len = targets.len;
        }
        let move_result = targets.count() > 0;
        if !move_result && !at_least_one_failure {
            self.state.pokemon[user.0 as usize].move_this_turn_result =
                crate::state::ResultFlag::Null;
        }
        if self.mflag(move_handle, rt::SPREAD_HIT) {
            let mut mons = [MonId::NONE; 4];
            for i in 0..targets.count() {
                mons[i] = targets.mon_at(i).expect("hit target");
            }
            self.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
                "spread",
                LogArg::Spread {
                    mons,
                    len: targets.len,
                },
            )));
        }
        Relay::Bool(move_result)
    }

    /// Ports sim/battle-actions.ts:617-638. Invulnerability failures retain
    /// false/null/NOT_FAIL distinctions before targeting is filtered.
    /// PRNG: none directly; Invulnerability handlers/order may draw.
    pub fn hit_step_invulnerability_event(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> Option<TargetResults> {
        let n = targets.count();
        let id = self.active_move(move_handle).id;
        if id == dex::MOVE_HELPINGHAND {
            return Some(TargetResults::filled(n, Relay::Bool(true)));
        }
        let mut results = TargetResults::filled(n, Relay::Undefined);
        for i in 0..n {
            let target = targets.mon_at(i).expect("hit target");
            // `target.volatiles['commanding']`: Commander is out of scope.
            let r = if id == dex::MOVE_TOXIC && self.has_type(user, &[POISON]) {
                Relay::Bool(true)
            } else {
                self.run_event(
                    EventId::Invulnerability,
                    mon_arg(target),
                    mon_arg(user),
                    EffectRef::ActiveMove(move_handle.0),
                    Relay::Undefined,
                    RunEventOptions::default(),
                )
            };
            results.set(i, r);
            if is_false(r) {
                if self.input_smart_target(MoveInput::Active(move_handle)) {
                    self.disable_smart_target(move_handle);
                } else {
                    if !self.mflag(move_handle, rt::SPREAD_HIT) {
                        self.attr_last_move(MoveLineEdit::Miss);
                    }
                    self.add(LogEntry::new(
                        "-miss",
                        &[LogArg::Mon(user), LogArg::Mon(target)],
                        &[],
                    ));
                }
            }
        }
        Some(results)
    }

    /// Ports sim/battle-actions.ts:639-649. Spread TryHit uses per-target relays.
    /// PRNG: none directly; TryHit handlers/order may draw.
    pub fn hit_step_try_hit_event(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> Option<TargetResults> {
        let n = targets.count();
        let mut mons = [MonId::NONE; 4];
        for i in 0..n {
            mons[i] = targets.mon_at(i).expect("hit target");
        }
        let mut results = self.run_event_spread(
            EventId::TryHit,
            EventTargets { mons, len: n as u8 },
            mon_arg(user),
            EffectRef::ActiveMove(move_handle.0),
            TargetResults::default(),
            RunEventOptions::default(),
        );
        let any_true = results.values[..results.count()]
            .iter()
            .any(|r| *r == Relay::Bool(true));
        if !any_true && results.includes_false() {
            self.log_fail(user);
            self.attr_last_move(MoveLineEdit::Still);
        }
        for i in 0..n {
            let r = results.at(i);
            if r != Relay::NotFail {
                results.set(i, if r.truthy() { r } else { Relay::FAIL });
            }
        }
        Some(results)
    }

    /// Ports sim/battle-actions.ts:650-661. Applies typed immunity unless the
    /// active move overrides it, retaining each target's result.
    /// PRNG: none directly; immunity callbacks may draw.
    pub fn hit_step_type_immunity(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> Option<TargetResults> {
        let _ = user;
        if !self.mflag(move_handle, rt::IGNORE_IMMUNITY_PRESENT) {
            let status = self.active_move(move_handle).category == dex::Category::Status;
            self.set_mflag(move_handle, rt::IGNORE_IMMUNITY_PRESENT, true);
            self.set_mflag(move_handle, rt::IGNORE_IMMUNITY, status);
        }
        let n = targets.count();
        let mut results = TargetResults::filled(n, Relay::Undefined);
        for i in 0..n {
            let target = targets.mon_at(i).expect("hit target");
            let message = if self.input_smart_target(MoveInput::Active(move_handle)) {
                ImmunityMessage::Silent
            } else {
                ImmunityMessage::Standard
            };
            let ok = self.run_immunity(target, ImmunitySource::Move(move_handle), message);
            results.set(i, Relay::Bool(ok));
        }
        Some(results)
    }

    /// Ports sim/battle-actions.ts:662-685. Calls the move's TryImmunity and
    /// handles Powder/Prankster immunity checks in source order.
    /// PRNG: none directly; dispatched callbacks may draw.
    pub fn hit_step_try_immunity(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> Option<TargetResults> {
        let n = targets.count();
        let mut results = TargetResults::filled(n, Relay::Undefined);
        let effect = EffectRef::ActiveMove(move_handle.0);
        for i in 0..n {
            let target = targets.mon_at(i).expect("hit target");
            let powder = self.mflag_flags(move_handle, dex::FLAG_POWDER);
            if powder && target != user && !self.dex_immunity(target, ImmunityId::Powder) {
                self.add(LogEntry::new("-immune", &[LogArg::Mon(target)], &[]));
                results.set(i, Relay::FAIL);
            } else if !self
                .single_event(
                    EventId::TryImmunity,
                    effect,
                    None,
                    mon_arg(target),
                    mon_arg(user),
                    effect,
                    Relay::Undefined,
                    None,
                )
                .truthy()
            {
                self.add(LogEntry::new("-immune", &[LogArg::Mon(target)], &[]));
                results.set(i, Relay::FAIL);
            } else if self.mflag(move_handle, rt::PRANKSTER_BOOSTED)
                && self.has_ability(user, &[dex::ABILITY_PRANKSTER])
                && !self.is_ally(user, Some(target))
                && !self.dex_immunity(target, ImmunityId::Prankster)
            {
                let status = self.active_move(move_handle).effects.status;
                if self.state.pokemon[target.0 as usize].illusion != MonId::NONE
                    || !(status != crate::ids::EffectId::NONE
                        && !self.dex_status_immunity(target, status))
                {
                    self.hint(
                        LogArg::Text("Since gen 7, Dark is immune to Prankster moves."),
                        false,
                        None,
                    );
                }
                self.add(LogEntry::new("-immune", &[LogArg::Mon(target)], &[]));
                results.set(i, Relay::FAIL);
            } else {
                results.set(i, Relay::Bool(true));
            }
        }
        Some(results)
    }

    /// `move.flags[...]` on the live overlay.
    fn mflag_flags(&self, move_handle: MoveHandle, flag: u64) -> bool {
        self.active_move(move_handle).flags & flag != 0
    }

    /// Ports sim/battle-actions.ts:686-750. Applies accuracy/evasion, OHKO rules,
    /// accuracy modifiers, misses, and Blunder Policy.
    /// PRNG: one randomChance(accuracy,100) per eligible target when accuracy is
    /// not true (line 734), including nominal 100%; events may draw additionally.
    pub fn hit_step_accuracy(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> Option<TargetResults> {
        let n = targets.count();
        let mut results = TargetResults::filled(n, Relay::Undefined);
        let effect = EffectRef::ActiveMove(move_handle.0);
        for i in 0..n {
            let target = targets.mon_at(i).expect("hit target");
            self.scratch.active_target = target;
            // Calculate true accuracy. OHKO moves and ignoreAccuracy/alwaysHit exist on
            // no scoped move, so only the ordinary branch is reachable.
            let mut accuracy = match self.active_move(move_handle).accuracy {
                MoveAccuracy::Always => Relay::Bool(true),
                MoveAccuracy::Percent(p) => Relay::Number(p),
            };
            accuracy = self.run_event(
                EventId::ModifyAccuracy,
                mon_arg(target),
                mon_arg(user),
                effect,
                accuracy,
                RunEventOptions::default(),
            );
            if accuracy != Relay::Bool(true) {
                // `move.ignoreAccuracy` exists on no scoped move.
                let boosts = self.modified_boosts(user);
                let mut boost = Self::boost_stage(&boosts, ACCURACY_STAGE);
                if !self.mflag(move_handle, rt::IGNORE_EVASION) {
                    let boosts = self.modified_boosts(target);
                    boost = clamp_int_range(
                        boost - Self::raw_stage(&boosts, EVASION_STAGE),
                        Some(-6.0),
                        Some(6.0),
                    );
                }
                let acc = relay_f64(accuracy);
                if boost > 0.0 {
                    accuracy = Relay::Number(f64::from(math::trunc_f64(acc * (3.0 + boost) / 3.0)));
                } else if boost < 0.0 {
                    accuracy = Relay::Number(f64::from(math::trunc_f64(acc * 3.0 / (3.0 - boost))));
                } else {
                    accuracy = Relay::Number(acc);
                }
            }
            let id = self.active_move(move_handle).id;
            let (kind, category) = {
                let m = self.active_move(move_handle);
                (m.target, m.category)
            };
            let always = (id == dex::MOVE_TOXIC && self.has_type(user, &[POISON]))
                || (kind == MoveTarget::SelfTarget
                    && category == dex::Category::Status
                    && !self.is_semi_invulnerable(target));
            if always {
                accuracy = Relay::Bool(true); // bypasses ohko accuracy modifiers
            } else {
                accuracy = self.run_event(
                    EventId::Accuracy,
                    mon_arg(target),
                    mon_arg(user),
                    effect,
                    accuracy,
                    RunEventOptions::default(),
                );
            }
            if accuracy != Relay::Bool(true) {
                let roll = self.state.prng.random(100);
                if !(f64::from(roll) < relay_f64(accuracy)) {
                    if self.input_smart_target(MoveInput::Active(move_handle)) {
                        self.disable_smart_target(move_handle);
                    } else {
                        if !self.mflag(move_handle, rt::SPREAD_HIT) {
                            self.attr_last_move(MoveLineEdit::Miss);
                        }
                        self.add(LogEntry::new(
                            "-miss",
                            &[LogArg::Mon(user), LogArg::Mon(target)],
                            &[],
                        ));
                    }
                    if self.has_item(user, &[dex::ITEM_BLUNDERPOLICY])
                        && self.use_item(user, Attribution::DEFAULT)
                    {
                        let mut spe = OrderedBoosts::default();
                        spe.values[4] = 2;
                        spe.order[0] = 4;
                        spe.len = 1;
                        spe.present = 1 << 4;
                        self.boost(spe, Some(user), Attribution::DEFAULT, false, false);
                    }
                    results.set(i, Relay::FAIL);
                    continue;
                }
            }
            results.set(i, Relay::Bool(true));
        }
        Some(results)
    }

    /// Ports sim/battle-actions.ts:751-776. Removes protection in the listed
    /// order; None preserves TS undefined for a stage with no result array.
    /// PRNG: none directly; End callbacks from removals may draw.
    pub fn hit_step_break_protect(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> Option<TargetResults> {
        if self.active_move(move_handle).traits & dex::MOVE_TRAIT_BREAKSPROTECT != 0 {
            // Scoped subset, in source order: kingsshield, obstruct and silktrap, and
            // craftyshield, matblock and quickguard are outside the generated Dex.
            const VOLATILES: [crate::ids::EffectId; 4] = [
                dex::CONDITION_BANEFULBUNKER,
                dex::CONDITION_BURNINGBULWARK,
                dex::CONDITION_PROTECT,
                dex::CONDITION_SPIKYSHIELD,
            ];
            for i in 0..targets.count() {
                let target = targets.mon_at(i).expect("hit target");
                let mut broke = false;
                for id in VOLATILES {
                    if self.remove_volatile(target, id) {
                        broke = true;
                    }
                }
                // `gen >= 6 || !target.isAlly(pokemon)`: always in Gen 9.
                if self.remove_side_condition(target.side(), dex::CONDITION_WIDEGUARD) {
                    broke = true;
                }
                if broke {
                    let id = self.active_move(move_handle).id;
                    if id == dex::MOVE_FEINT {
                        self.add(LogEntry::new(
                            "-activate",
                            &[LogArg::Mon(target), LogArg::Text("move: Feint")],
                            &[],
                        ));
                    } else {
                        self.add(LogEntry::new(
                            "-activate",
                            &[
                                LogArg::Mon(target),
                                LogArg::EffectFullName(EffectRef::ActiveMove(move_handle.0)),
                            ],
                            &[LogTag::Bare("broken")],
                        ));
                    }
                    // `delete target.volatiles['stall']`: a raw delete, no End event.
                    self.delete_volatile_raw(target, dex::CONDITION_STALL);
                }
            }
        }
        let _ = user;
        None
    }

    /// `delete pokemon.volatiles[id]`: drop the entry without any End callback.
    pub(super) fn delete_volatile_raw(&mut self, pokemon: MonId, id: crate::ids::EffectId) {
        let found = self.state.pokemon[pokemon.0 as usize]
            .volatiles
            .as_slice()
            .iter()
            .position(|c| self.state.effects.cells[c.0 as usize].id == id);
        if let Some(index) = found {
            let cell = self.state.pokemon[pokemon.0 as usize]
                .volatiles
                .remove(index);
            self.state.effects.release(cell);
        }
    }

    /// Ports sim/battle-actions.ts:777-806. Steals positive boosts before hits,
    /// using insertion-ordered boosts and clearing exactly those target stats.
    /// PRNG: none directly; boost mutation events may draw.
    pub fn hit_step_steal_boosts(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> Option<TargetResults> {
        // `move.stealsBoosts` exists only on Spectral Thief (isNonstandard Past), which is not
        // in the generated Dex, so this stage always returns undefined.
        let _ = (targets, user, move_handle);
        None
    }

    /// Ports sim/battle-actions.ts:807-812. Dispatches AfterMoveSecondary once
    /// spread-hit processing has finished and secondaries are not suppressed.
    /// PRNG: none directly; dispatched callbacks/order may draw.
    pub fn after_move_secondary_event(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) {
        let effect = EffectRef::ActiveMove(move_handle.0);
        self.single_event(
            EventId::AfterMoveSecondary,
            effect,
            None,
            opt_mon_arg_undef(targets.mon_at(0)),
            mon_arg(user),
            effect,
            Relay::Undefined,
            None,
        );
        let n = targets.count();
        let mut mons = [MonId::NONE; 4];
        for i in 0..n {
            mons[i] = targets.mon_at(i).expect("hit target");
        }
        self.run_event_spread(
            EventId::AfterMoveSecondary,
            EventTargets { mons, len: n as u8 },
            mon_arg(user),
            effect,
            TargetResults::default(),
            RunEventOptions::default(),
        );
    }

    /// Ports sim/battle-actions.ts:813-849. Field and side-target moves use
    /// TryHitField/TryHitSide, then the primary resolution helper.
    /// PRNG: none directly; nested hit processing and callbacks may draw.
    pub fn try_move_hit(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> Relay {
        let target = targets.mon_at(0).expect("field/side move without a target");
        self.set_active_move(Some(move_handle), Some(user), Some(target));
        let effect = EffectRef::ActiveMove(move_handle.0);
        let hit_result = self.try_prepare_hit(move_handle, user, Some(target));
        if !hit_result.truthy() {
            if is_false(hit_result) {
                self.log_fail(user);
                self.attr_last_move(MoveLineEdit::Still);
            }
            return Relay::FAIL;
        }
        // Free-for-all hazards are outside this format.
        let event = if self.active_move(move_handle).target == MoveTarget::All {
            EventId::TryHitField
        } else {
            EventId::TryHitSide
        };
        let hit_result = self.run_event(
            event,
            mon_arg(target),
            mon_arg(user),
            effect,
            Relay::Undefined,
            RunEventOptions::default(),
        );
        if !hit_result.truthy() {
            if is_false(hit_result) {
                self.log_fail(user);
                self.attr_last_move(MoveLineEdit::Still);
            }
            return Relay::FAIL;
        }
        self.move_hit(
            Targets::from_mons(&[target]),
            user,
            move_handle,
            HitEffect::Primary,
            HitOptions::default(),
        )
    }

    /// Ports sim/battle-actions.ts:850-1013. Runs per-hit spread resolution,
    /// repeated accuracy, recoil, faint flushing, and multihit log annotations.
    /// PRNG: 2-5-hit sample (863), optional Loaded Dice correction draw (865),
    /// generic inclusive range (871), Population Bomb/Loaded Dice draw (874),
    /// repeated accuracy (931), each damage/secondary draw, and callback draws.
    pub fn hit_step_move_hit_loop(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> Option<TargetResults> {
        let n = targets.count();
        let effect = EffectRef::ActiveMove(move_handle.0);
        let mut damage = TargetResults::filled(n, Relay::Number(0.0));
        self.active_move_mut(move_handle).total_damage = 0;
        let mut target_hits: u32 = 1; // move.multihit || 1
        if self.mflag(move_handle, rt::MULTIHIT_PRESENT) {
            let [low, high] = self.active_move(move_handle).multihit;
            if self.mflag(move_handle, rt::MULTIHIT_RANGE) {
                // Yes, it's hardcoded in the source.
                if low == 2 && high == 5 {
                    let i = self.state.prng.sample_index(MULTIHIT_TABLE.len());
                    target_hits = u32::from(MULTIHIT_TABLE[i]);
                    if target_hits < 4 && self.has_item(user, &[dex::ITEM_LOADEDDICE]) {
                        target_hits = 5 - self.state.prng.random(2);
                    }
                } else {
                    target_hits = self
                        .state
                        .prng
                        .random_range(u32::from(low), u32::from(high) + 1);
                }
            } else {
                target_hits = u32::from(low);
            }
        }
        if target_hits == 10 && self.has_item(user, &[dex::ITEM_LOADEDDICE]) {
            target_hits -= self.state.prng.random(7);
        }
        let mut null_damage = true;
        let mut move_damage = TargetResults::default();
        // Sleep Talk and Snore (`sleepUsable`) are not in the generated Dex.
        let is_sleep_usable = false;

        let mut targets_copy = targets;
        let mut hit: u32 = 1;
        while hit <= target_hits {
            if damage.includes_false() {
                break;
            }
            if hit > 1
                && self.state.pokemon[user.0 as usize].status == Status::Sleep
                && !is_sleep_usable
            {
                break;
            }
            if (0..n).all(|i| targets.mon_at(i).is_none_or(|t| self.mon_hp(t) == 0)) {
                break;
            }
            {
                let m = self.active_move_mut(move_handle);
                m.hit = hit as u8;
            }
            self.set_mflag(move_handle, rt::LAST_HIT, hit == target_hits);
            let smart = self.input_smart_target(MoveInput::Active(move_handle));
            if smart && n > 1 {
                let which = (hit - 1) as usize;
                targets_copy = Targets::single(match targets.mon_at(which) {
                    Some(m) => HitTarget::Pokemon(m),
                    None => HitTarget::False,
                });
                damage = TargetResults::filled(1, damage.at(which));
            } else {
                targets_copy = targets;
            }
            // Some relevant-to-single-target-moves-only things are hardcoded.
            let target = targets_copy.mon_at(0);
            if let Some(t) = target {
                if self.mflag(move_handle, rt::SMART_TARGET_PRESENT) {
                    if hit > 1 {
                        self.add_move(LogEntry::new(
                            "-anim",
                            &[LogArg::Mon(user), LogArg::Effect(effect), LogArg::Mon(t)],
                            &[],
                        ));
                    } else {
                        self.attr_last_move(MoveLineEdit::Retarget(t));
                    }
                }
            }

            // Like this (Triple Axel).
            if let Some(t) = target {
                if self.mflag(move_handle, rt::MULTIACCURACY) && hit > 1 {
                    let mut accuracy = match self.active_move(move_handle).accuracy {
                        MoveAccuracy::Always => Relay::Bool(true),
                        MoveAccuracy::Percent(p) => Relay::Number(p),
                    };
                    if accuracy != Relay::Bool(true) {
                        let mut acc = relay_f64(accuracy);
                        let boosts = self.modified_boosts(user);
                        let boost = Self::boost_stage(&boosts, ACCURACY_STAGE);
                        if boost > 0.0 {
                            acc *= BOOST_TABLE[boost as usize];
                        } else {
                            acc /= BOOST_TABLE[(-boost) as usize];
                        }
                        if !self.mflag(move_handle, rt::IGNORE_EVASION) {
                            let boosts = self.modified_boosts(t);
                            let boost = Self::boost_stage(&boosts, EVASION_STAGE);
                            if boost > 0.0 {
                                acc /= BOOST_TABLE[boost as usize];
                            } else if boost < 0.0 {
                                acc *= BOOST_TABLE[(-boost) as usize];
                            }
                        }
                        accuracy = Relay::Number(acc);
                    }
                    accuracy = self.run_event(
                        EventId::ModifyAccuracy,
                        mon_arg(t),
                        mon_arg(user),
                        effect,
                        accuracy,
                        RunEventOptions::default(),
                    );
                    // `move.alwaysHit` exists on no scoped move.
                    accuracy = self.run_event(
                        EventId::Accuracy,
                        mon_arg(t),
                        mon_arg(user),
                        effect,
                        accuracy,
                        RunEventOptions::default(),
                    );
                    if accuracy != Relay::Bool(true) {
                        let roll = self.state.prng.random(100);
                        if !(f64::from(roll) < relay_f64(accuracy)) {
                            break;
                        }
                    }
                }
            }

            // Modifies targetsCopy (which is why it's a copy).
            let (move_damage_this_hit, copy_after) = self.spread_move_hit(
                targets_copy,
                user,
                move_handle,
                HitEffect::Primary,
                HitOptions::default(),
            );
            targets_copy = copy_after;
            // When Dragon Darts targets two different pokemon, targetsCopy is a length 1 array
            // each hit so spreadMoveHit returns a length 1 damage array.
            if self.input_smart_target(MoveInput::Active(move_handle)) {
                for i in 0..move_damage_this_hit.count() {
                    move_damage.push_value(move_damage_this_hit.at(i));
                }
            } else {
                move_damage = move_damage_this_hit;
            }

            if !(0..move_damage.count()).any(|i| !is_false(move_damage.at(i))) {
                break;
            }
            null_damage = false;

            let smart_now = self.input_smart_target(MoveInput::Active(move_handle));
            for i in 0..move_damage.count() {
                if smart_now && i != (hit - 1) as usize {
                    continue;
                }
                // Damage from each hit is individually counted for the purposes of Counter,
                // Metal Burst, and Mirror Coat.
                let md = move_damage.at(i);
                let value = if md == Relay::Bool(true) || !md.truthy() {
                    0.0
                } else {
                    relay_f64(md)
                };
                damage.set(i, Relay::Number(value));
                // Total damage dealt is accumulated for the purposes of recoil.
                let m = self.active_move_mut(move_handle);
                m.total_damage = m.total_damage.wrapping_add(math::trunc_f64(value));
            }
            self.each_event(EventId::Update, EffectRef::None, Relay::Undefined);
            if self.mon_hp(user) == 0 && n == 1 {
                hit += 1; // report the correct number of hits for multihit moves
                break;
            }
            hit += 1;
        }
        // `hit` is 1 higher than the actual hit count.
        if hit == 1 {
            for i in 0..damage.count() {
                damage.set(i, Relay::FAIL);
            }
            return Some(damage);
        }
        if null_damage {
            for i in 0..damage.count() {
                damage.set(i, Relay::FAIL);
            }
        }
        let user_fainted = self.mon_hp(user) == 0;
        self.faint_messages(false, false, user_fainted);
        if self.mflag(move_handle, rt::MULTIHIT_PRESENT)
            && !self.mflag(move_handle, rt::SMART_TARGET_PRESENT)
        {
            let first = targets.mon_at(0).expect("hit target");
            self.add(LogEntry::new(
                "-hitcount",
                &[LogArg::Mon(first), LogArg::Number(hit as i32 - 1)],
                &[],
            ));
        }

        let total = self.active_move(move_handle).total_damage;
        if total != 0 {
            self.apply_recoil_damage(f64::from(total), move_handle, user);
        }

        // smartTarget messes up targetsCopy, but smartTarget should in theory ensure that
        // targets will never fail, anyway.
        let smart = self.input_smart_target(MoveInput::Active(move_handle));
        if smart {
            targets_copy = targets;
        }

        let move_id = self.active_move(move_handle).id;
        for i in 0..targets_copy.count() {
            if let Some(t) = targets_copy.mon_at(i) {
                if t != user {
                    let dealt = move_damage.at(i);
                    self.got_attacked(t, move_id, dealt, user);
                    if matches!(dealt, Relay::Number(_)) {
                        self.state.pokemon[t.0 as usize].times_attacked +=
                            if smart { 1 } else { hit - 1 };
                    }
                }
            }
        }

        // `move.ohko` exists on no scoped move.
        if !(0..damage.count()).any(|i| keeps(damage.at(i))) {
            return Some(damage);
        }

        self.each_event(EventId::Update, EffectRef::None, Relay::Undefined);

        if !self.suppressing_secondaries() {
            let mut surviving = Targets::default();
            for i in 0..targets_copy.count() {
                if let Some(t) = targets_copy.mon_at(i) {
                    surviving.push_target(HitTarget::Pokemon(t));
                }
            }
            self.after_move_secondary_event(surviving, user, move_handle);

            for i in 0..damage.count() {
                // There are no multihit spread moves, so it's safe to use move.totalDamage for
                // multihit moves. The previous check was for `move.multihit`, but that fails
                // for Dragon Darts.
                let cur = if n == 1 {
                    Relay::Number(f64::from(self.active_move(move_handle).total_damage))
                } else {
                    damage.at(i)
                };
                let Some(t) = targets.mon_at(i) else {
                    continue;
                };
                if let Relay::Number(cur) = cur {
                    if self.mon_hp(t) != 0 {
                        // `hurtThisTurn` is not kept in the Copy state (no scoped
                        // EmergencyExit listener reads it); the damage is the relay value.
                        self.run_event(
                            EventId::EmergencyExit,
                            mon_arg(t),
                            mon_arg(user),
                            EffectRef::None,
                            Relay::Number(cur),
                            RunEventOptions::default(),
                        );
                    }
                }
            }
        }
        Some(damage)
    }

    /// Ports sim/battle-actions.ts:1014-1126. Resolves substitute, damage, hit
    /// effects, self/secondary effects, force-switch, and DamagingHit events.
    /// False targets stop; Null targets continue self effects after Substitute.
    /// PRNG: damage/self/secondary/event draws only, in source step order.
    pub fn spread_move_hit(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
        hit_effect: HitEffect,
        options: HitOptions,
    ) -> (TargetResults, Targets) {
        let mut targets = targets;
        let n = targets.count();
        let (is_secondary, is_self) = (options.secondary, options.self_hit);
        let effect = EffectRef::ActiveMove(move_handle.0);
        // Hardcoded for single-target purposes (no spread moves have any kind of onTryHit).
        let first = targets.at(0);
        let mut damage = TargetResults::filled(n, Relay::Bool(true));
        let kind = self.active_move(move_handle).target;
        let view = self.hit_view(move_handle, hit_effect);
        let mut hit_result = Relay::Bool(true);
        let target_arg = match first {
            HitTarget::Pokemon(m) => mon_arg(m),
            _ => EventArg::Null,
        };
        if kind == MoveTarget::All && !is_self {
            hit_result =
                self.hit_single_event(&view, EventId::TryHitField, move_handle, target_arg, user);
        } else if matches!(kind, MoveTarget::FoeSide | MoveTarget::AllySide) && !is_self {
            hit_result =
                self.hit_single_event(&view, EventId::TryHitSide, move_handle, target_arg, user);
        } else if let HitTarget::Pokemon(t) = first {
            hit_result =
                self.hit_single_event(&view, EventId::TryHit, move_handle, mon_arg(t), user);
        }
        if !hit_result.truthy() {
            if is_false(hit_result) {
                self.log_fail(user);
                self.attr_last_move(MoveLineEdit::Still);
            }
            return (TargetResults::filled(1, Relay::FAIL), targets); // single-target only
        }

        // 0. check for substitute
        if !is_secondary
            && !is_self
            && !matches!(
                kind,
                MoveTarget::All | MoveTarget::AllySide | MoveTarget::FoeSide
            )
        {
            damage =
                self.try_primary_hit_event(damage, targets, user, move_handle, hit_effect, options);
        }
        let view = self.hit_view(move_handle, hit_effect);
        for i in 0..n {
            if damage.at(i) == Relay::HIT_SUBSTITUTE {
                damage.set(i, Relay::Bool(true));
                targets.entries[i] = HitTarget::Null;
            }
            if matches!(targets.at(i), HitTarget::Pokemon(_)) && is_secondary && !view.has_self {
                damage.set(i, Relay::Bool(true));
            }
            if !damage.at(i).truthy() {
                targets.entries[i] = HitTarget::False;
            }
        }
        // 1. call to getDamage
        damage = self.spread_damage_numbers(damage, targets, user, move_handle, hit_effect);
        for i in 0..n {
            if is_false(damage.at(i)) {
                targets.entries[i] = HitTarget::False;
            }
        }
        // 2. call to spreadDamage
        damage = self.spread_damage(damage, targets, Attribution::from_move(user, effect), false);
        for i in 0..n {
            if is_false(damage.at(i)) {
                targets.entries[i] = HitTarget::False;
            }
        }
        // 3. onHit event happens here
        damage = self.run_move_effects(damage, targets, user, move_handle, hit_effect, options);
        for i in 0..n {
            if failed_not_zero(damage.at(i)) {
                targets.entries[i] = HitTarget::False;
            }
        }

        // Steps 4 and 5 can mess with battle.activeTarget, which needs to be preserved for Dancer.
        let active_target = self.scratch.active_target;
        // 4. self drops (start checking for targets[i] === false here)
        let view = self.hit_view(move_handle, hit_effect);
        if view.has_self && !self.mflag(move_handle, rt::SELF_DROPPED) {
            self.self_drops(targets, user, move_handle, hit_effect, is_secondary);
        }
        // 5. secondary effects
        let view = self.hit_view(move_handle, hit_effect);
        if view.has_secondaries {
            self.secondaries(targets, user, move_handle, hit_effect, is_self);
        }
        self.scratch.active_target = active_target;
        // 6. force switch
        let view = self.hit_view(move_handle, hit_effect);
        if view.effects.force_switch {
            damage = self.force_switch(damage, targets, user, move_handle);
        }
        for i in 0..n {
            if failed_not_zero(damage.at(i)) {
                targets.entries[i] = HitTarget::False;
            }
        }

        let mut damaged = EventTargets {
            mons: [MonId::NONE; 4],
            len: 0,
        };
        let mut damaged_damage = TargetResults::default();
        for i in 0..n {
            if let (Relay::Number(_), HitTarget::Pokemon(t)) = (damage.at(i), targets.at(i)) {
                damaged.mons[damaged.len as usize] = t;
                damaged.len += 1;
                damaged_damage.push_value(damage.at(i));
            }
        }
        let pokemon_original_hp = self.mon_hp(user);
        if damaged.len != 0 && !is_secondary && !is_self {
            self.run_event_spread(
                EventId::DamagingHit,
                damaged,
                mon_arg(user),
                effect,
                damaged_damage,
                RunEventOptions::default(),
            );
            let view = self.hit_view(move_handle, hit_effect);
            if self.hit_hook(&view, EventId::AfterHit).is_some() && self.mon_hp(user) != 0 {
                for k in 0..damaged.len as usize {
                    let view = self.hit_view(move_handle, hit_effect);
                    self.hit_single_event(
                        &view,
                        EventId::AfterHit,
                        move_handle,
                        mon_arg(damaged.mons[k]),
                        user,
                    );
                }
            }
            self.run_event(
                EventId::EmergencyExit,
                mon_arg(user),
                EventArg::Undefined,
                EffectRef::None,
                Relay::Number(f64::from(pokemon_original_hp)),
                RunEventOptions::default(),
            );
        }
        (damage, targets)
    }

    /// `getSpreadDamage` (battle-actions.ts:1137-1174). Only the move itself has a base power
    /// or damage callback; for secondaries, `self` and `selfBoost` getDamage returns undefined
    /// without touching anything, so only `activeTarget` moves.
    fn spread_damage_numbers(
        &mut self,
        damage: TargetResults,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
        hit_effect: HitEffect,
    ) -> TargetResults {
        let mut damage = damage;
        if matches!(hit_effect, HitEffect::Primary) {
            // D keeps the incoming relay for targets that are already null/false.
            damage = self.get_spread_damage_from(damage, targets, user, move_handle);
        } else {
            for i in 0..targets.count() {
                if let Some(t) = targets.mon_at(i) {
                    self.scratch.active_target = t;
                    damage.set(i, Relay::Undefined);
                }
            }
        }
        damage
    }

    /// Ports sim/battle-actions.ts:1127-1136. Runs TryPrimaryHit for each present
    /// target; the numeric zero sentinel represents Substitute absorption.
    /// PRNG: none directly; dispatched callbacks/order may draw.
    pub fn try_primary_hit_event(
        &mut self,
        damage: TargetResults,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
        hit_effect: HitEffect,
        options: HitOptions,
    ) -> TargetResults {
        let _ = (hit_effect, options);
        let mut damage = damage;
        for i in 0..targets.count() {
            let Some(target) = targets.mon_at(i) else {
                continue;
            };
            let r = self.run_event(
                EventId::TryPrimaryHit,
                mon_arg(target),
                mon_arg(user),
                EffectRef::ActiveMove(move_handle.0),
                Relay::Undefined,
                RunEventOptions::default(),
            );
            damage.set(i, r);
        }
        damage
    }

    /// Ports sim/battle-actions.ts:1359-1366. Adapts spread resolution for effect
    /// callbacks; true becomes undefined while zero remains numeric zero.
    /// PRNG: delegated spread-hit pipeline and callbacks may draw.
    pub fn move_hit(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
        hit_effect: HitEffect,
        options: HitOptions,
    ) -> Relay {
        let (damage, _) = self.spread_move_hit(targets, user, move_handle, hit_effect, options);
        let first = damage.at(0);
        if first == Relay::Bool(true) {
            Relay::Undefined
        } else {
            first
        }
    }
}

/// Event argument for a possibly-missing Pokemon where JS passes `undefined`
/// (an empty `targets[0]`), not `null`.
fn opt_mon_arg_undef(m: Option<MonId>) -> EventArg {
    m.map_or(EventArg::Undefined, mon_arg)
}
