//! Gen 9 doubles location, target, and redirection rules. OWNER M.
use super::support::*;
use crate::{
    actions::{HitTarget, MoveHandle, MoveInput, MoveTargets, Targets},
    battle::Battle,
    dex::{self, MoveTarget},
    event::{EffectRef, Relay},
    ids::{EventId, MonId, SideId, SlotId},
    log::{LogSink, MoveLineEdit},
    state::scratch::move_runtime as rt,
};

impl<L: LogSink> Battle<L> {
    // side.ts:390-410 filters absent slots, then HP (not the fainted flag).
    // Only adjacentAllies additionally calls isAdjacent (pokemon.ts:741-746).
    fn active_side_targets(
        &self,
        side: SideId,
        all: bool,
        exclude: Option<MonId>,
        adjacent: Option<MonId>,
    ) -> Targets {
        let mut result = Targets::default();
        for mon in self.state.sides[side.0 as usize].active {
            if mon == MonId::NONE || Some(mon) == exclude {
                continue;
            }
            let pokemon = &self.state.pokemon[mon.0 as usize];
            if !all && pokemon.hp == 0 {
                continue;
            }
            if let Some(user) = adjacent {
                if user == mon
                    || pokemon.flags & crate::state::mon_flags::FAINTED != 0
                    || self.state.pokemon[user.0 as usize].flags & crate::state::mon_flags::FAINTED
                        != 0
                {
                    continue;
                }
            }
            result.entries[result.len as usize] = HitTarget::Pokemon(mon);
            result.len += 1;
        }
        result
    }

    /// Shared body of validTargetLoc. `None` is the Recharge pseudo move, whose
    /// `target` is undefined: only the zero location is valid (battle.ts:2403-2436).
    pub(super) fn valid_target_loc_kind(
        &self,
        target_loc: i8,
        source: MonId,
        target_type: Option<MoveTarget>,
    ) -> bool {
        if target_loc == 0 {
            return true;
        }
        const NUM_SLOTS: i8 = 2; // activePerHalf in doubles
        let source_loc = self.get_loc_of(source, source);
        if target_loc.abs() > NUM_SLOTS {
            return false;
        }
        let is_self = source_loc == target_loc;
        let is_foe = target_loc > 0;
        let across_from_target_loc = -(NUM_SLOTS + 1 - target_loc);
        let is_adjacent = if target_loc > 0 {
            (across_from_target_loc - source_loc).abs() <= 1
        } else {
            (target_loc - source_loc).abs() == 1
        };
        match target_type {
            // 'scripted', 'adjacentAllyOrSelf' and 'adjacentFoe' are not in the scoped dex.
            Some(MoveTarget::RandomNormal | MoveTarget::Normal) => is_adjacent,
            Some(MoveTarget::AdjacentAlly) => is_adjacent && !is_foe,
            Some(MoveTarget::Any) => !is_self,
            _ => false,
        }
    }

    /// Ports sim/battle.ts:2403-2436. Checks signed locations against move target
    /// kind without resolving an occupant; used by the text validator.
    /// PRNG: none.
    pub fn valid_target_loc(&self, target_loc: i8, source: MonId, target_type: MoveTarget) -> bool {
        self.valid_target_loc_kind(target_loc, source, Some(target_type))
    }

    /// Ports sim/battle.ts:2437-2439. Location-based legality for a concrete mon.
    /// PRNG: none.
    pub fn valid_target(&self, target: MonId, source: MonId, target_type: MoveTarget) -> bool {
        self.valid_target_loc(self.get_loc_of(source, target), source, target_type)
    }

    /// Ports sim/battle.ts:2441-2492. Tracks original targets, validates selected
    /// positions, preserves fainted ally targets, and applies fallback targeting.
    /// A missing location (JS undefined) behaves exactly like zero here: it never
    /// equals the user's location and never names an occupant.
    /// PRNG: fallback get_random_target samples; no draw for a valid selected target.
    pub fn get_target(
        &mut self,
        user: MonId,
        move_input: MoveInput,
        target_loc: Option<i8>,
        original_target: Option<MonId>,
    ) -> Option<MonId> {
        let kind = self.input_target(move_input);
        let mut tracks_target = self.input_tracks_target(move_input);
        // Stalwart sets trackTarget in ModifyMove, but ModifyMove happens after getTarget, so
        // we need to manually check for Stalwart here.
        if self.has_ability(user, &[dex::ABILITY_STALWART, dex::ABILITY_PROPELLERTAIL]) {
            tracks_target = true;
        }
        if tracks_target {
            if let Some(original) = original_target {
                if self.mon_active(original) {
                    // Smart-tracking move's original target is on the field: target it.
                    return Some(original);
                }
            }
        }
        let loc = target_loc.unwrap_or(0);
        // Banning Dragon Darts from directly targeting itself is done in side.ts.
        if self.input_smart_target(move_input) {
            let cur = self.get_at_loc(user, loc);
            return match cur {
                Some(m) if !self.mon_fainted(m) => Some(m),
                _ => self.get_random_target(user, move_input),
            };
        }
        // Fails if the target is the user and the move can't target its own position.
        let self_loc = self.get_loc_of(user, user);
        if matches!(
            kind,
            Some(MoveTarget::AdjacentAlly | MoveTarget::Any | MoveTarget::Normal)
        ) && target_loc == Some(self_loc)
            && self
                .get_volatile(user, dex::CONDITION_TWOTURNMOVE)
                .is_none()
        {
            // iceball and rollout are outside the scoped Dex; 'futuremove' is no scoped flag.
            return None;
        }
        if kind != Some(MoveTarget::RandomNormal) && self.valid_target_loc_kind(loc, user, kind) {
            let target = self.get_at_loc(user, loc);
            if let Some(t) = target {
                if self.mon_fainted(t) {
                    // Free-for-all and adjacentAllyOrSelf are not in this format.
                    if self.is_ally(user, Some(t)) {
                        // Target is a fainted ally: attack shouldn't retarget.
                        return Some(t);
                    }
                }
                if !self.mon_fainted(t) {
                    // Target is unfainted: use selected target location.
                    return Some(t);
                }
            }
            // Chosen target not valid, retarget randomly with getRandomTarget.
        }
        self.get_random_target(user, move_input)
    }

    /// Ports sim/battle.ts:2494-2530 and sim/side.ts:365-372. Automatic targeting
    /// samples eligible foes/allies in array order, including singleton samples.
    /// PRNG: exactly one sample draw when a nonempty candidate list is sampled;
    /// self/side/field target kinds and empty-list fallback make no draw.
    pub fn get_random_target(&mut self, user: MonId, move_input: MoveInput) -> Option<MonId> {
        // A move was used without a chosen target. Moves that can target either allies or
        // foes only target foes when used without an explicit target.
        match self.input_target(move_input) {
            Some(MoveTarget::SelfTarget | MoveTarget::All | MoveTarget::AllySide) => {
                return Some(user);
            }
            Some(MoveTarget::AdjacentAlly) => {
                let allies = self.adjacent_allies(user);
                if allies.len == 0 {
                    return None;
                }
                let i = self.state.prng.sample_index(allies.len as usize);
                return allies.mon_at(i);
            }
            _ => {}
        }
        // Doubles: side.randomFoe() || side.foe.active[0].
        let foes = self.foes(user, false);
        if foes.len > 0 {
            let i = self.state.prng.sample_index(foes.len as usize);
            return foes.mon_at(i);
        }
        let first = self.state.sides[(user.side().0 ^ 1) as usize].active[0];
        (first != MonId::NONE).then_some(first)
    }

    /// Ports sim/pokemon.ts:791-854. Collects hit targets and separately Pressure
    /// targets, then applies RedirectTarget and smart/spread targeting rules.
    /// PRNG: a fainted foe can cause get_random_target to sample; redirection
    /// handler ordering/callbacks may draw additionally.
    pub fn get_move_targets(
        &mut self,
        user: MonId,
        move_handle: MoveHandle,
        target: MonId,
    ) -> MoveTargets {
        let kind = self.active_move(move_handle).target;
        let mut targets = Targets::default();
        match kind {
            MoveTarget::All | MoveTarget::FoeSide | MoveTarget::AllySide => {
                if kind != MoveTarget::FoeSide {
                    targets.extend_targets(self.allies_and_self(user));
                }
                if kind != MoveTarget::AllySide {
                    targets.extend_targets(self.foes(user, true));
                }
                if targets.len > 0 && !targets.contains_mon(target) {
                    let last = targets
                        .mon_at(targets.len as usize - 1)
                        .expect("pokemon target");
                    self.attr_last_move(MoveLineEdit::Retarget(last));
                }
            }
            MoveTarget::AllAdjacent | MoveTarget::AllAdjacentFoes => {
                if kind == MoveTarget::AllAdjacent {
                    targets.extend_targets(self.adjacent_allies(user));
                }
                targets.extend_targets(self.adjacent_foes(user));
                if targets.len > 0 && !targets.contains_mon(target) {
                    let last = targets
                        .mon_at(targets.len as usize - 1)
                        .expect("pokemon target");
                    self.attr_last_move(MoveLineEdit::Retarget(last));
                }
            }
            MoveTarget::Allies => {
                targets = self.allies_and_self(user);
            }
            _ => {
                let selected_target = target;
                let mut target = target;
                // If a targeted foe faints, the move is retargeted.
                if self.mon_fainted(target) && !self.is_ally(user, Some(target)) {
                    match self.get_random_target(user, MoveInput::Active(move_handle)) {
                        Some(possible) => target = possible,
                        None => return MoveTargets::default(),
                    }
                }
                if !self.mflag(move_handle, rt::TRACKS_TARGET) {
                    target = self.redirect_target(user, target, move_handle);
                }
                if self.input_smart_target(MoveInput::Active(move_handle)) {
                    targets = self.get_smart_targets(user, target, move_handle);
                    target = targets.mon_at(0).expect("smart target list");
                } else {
                    targets.push_target(HitTarget::Pokemon(target));
                }
                // 'futuremove' is not a scoped flag.
                if self.mon_fainted(target) {
                    return MoveTargets::default();
                }
                if selected_target != target {
                    self.attr_last_move(MoveLineEdit::Retarget(target));
                }
            }
        }
        // Resolve apparent targets for Pressure.
        let mut pressure = targets;
        if kind == MoveTarget::FoeSide {
            pressure = Targets::default();
        }
        if self.active_move(move_handle).flags & dex::FLAG_MUSTPRESSURE != 0 {
            pressure = self.foes(user, false);
        }
        MoveTargets {
            hit: targets,
            pressure,
        }
    }

    /// Ports RedirectTarget in sim/pokemon.ts:827-829. Handler order belongs to
    /// priorityEvent, including the source move's priority and effectOrder rules.
    /// PRNG: none directly; event ordering/callbacks may draw.
    pub fn redirect_target(
        &mut self,
        user: MonId,
        target: MonId,
        move_handle: MoveHandle,
    ) -> MonId {
        match self.priority_event(
            EventId::RedirectTarget,
            mon_arg(user),
            mon_arg(user),
            EffectRef::ActiveMove(move_handle.0),
            Relay::Pokemon(target),
            false,
        ) {
            Relay::Pokemon(m) => m,
            other => panic!("RedirectTarget returned a non-Pokemon relay {other:?}"),
        }
    }

    /// Ports sim/pokemon.ts:770-782. Resolves a signed relative active position.
    /// PRNG: none.
    pub fn get_at_loc(&self, user: MonId, target_loc: i8) -> Option<MonId> {
        // sides[loc < 0 ? n % 2 : (n + 1) % 2]; a location past the side's slots would
        // index sides[2], which does not exist in this two-side format.
        let own = user.side().0;
        let side = if target_loc < 0 { own } else { own ^ 1 };
        let loc = target_loc.unsigned_abs() as usize;
        assert!(
            loc <= 2,
            "location {target_loc} reaches a nonexistent ally side"
        );
        if loc == 0 {
            return None; // active[-1]
        }
        let mon = self.state.sides[side as usize].active[loc - 1];
        (mon != MonId::NONE).then_some(mon)
    }

    /// Ports sim/pokemon.ts:784-789. Converts a current position to signed loc.
    /// PRNG: none.
    pub fn get_loc_of(&self, user: MonId, target: MonId) -> i8 {
        let position = self.state.pokemon[target.0 as usize].position as i8 + 1;
        if user.side() == target.side() {
            -position
        } else {
            position
        }
    }

    /// Ports sim/pokemon.ts:520-527. The slot follows current party position;
    /// its encoding differs from stable MonId and cross-side field rank.
    /// PRNG: none.
    pub fn get_slot(&self, mon: MonId) -> SlotId {
        SlotId::new(mon.side(), self.state.pokemon[mon.0 as usize].position)
    }

    /// Ports sim/pokemon.ts:737-739. Checks side/team affiliation.
    /// PRNG: none.
    pub fn is_ally(&self, mon: MonId, other: Option<MonId>) -> bool {
        other.is_some_and(|o| mon.side() == o.side())
    }

    /// Ports sim/pokemon.ts:741-747. Gen 9 doubles positional adjacency.
    /// PRNG: none.
    pub fn is_adjacent(&self, mon: MonId, other: MonId) -> bool {
        if self.mon_fainted(mon) || self.mon_fainted(other) {
            return false;
        }
        mon != other // activePerHalf <= 2
    }

    /// Ports sim/pokemon.ts:757-768. Dragon Darts includes the target's adjacent
    /// ally when eligible; otherwise disables smartTarget on the mutable move.
    /// PRNG: none.
    pub fn get_smart_targets(
        &mut self,
        user: MonId,
        target: MonId,
        move_handle: MoveHandle,
    ) -> Targets {
        let target2 = self.adjacent_allies(target).mon_at(0);
        match target2 {
            Some(t2) if t2 != user && self.mon_hp(t2) != 0 => {
                if self.mon_hp(target) == 0 {
                    self.disable_smart_target(move_handle);
                    return Targets::from_mons(&[t2]);
                }
                Targets::from_mons(&[target, t2])
            }
            _ => {
                self.disable_smart_target(move_handle);
                Targets::from_mons(&[target])
            }
        }
    }

    /// `move.smartTarget = false`: present and false (not deleted).
    pub(super) fn disable_smart_target(&mut self, move_handle: MoveHandle) {
        let m = self.active_move_mut(move_handle);
        m.runtime_flags |= rt::SMART_TARGET_PRESENT;
        m.runtime_flags &= !rt::SMART_TARGET;
    }

    /// Ports sim/pokemon.ts:716-718 and sim/side.ts:390-395. Returns living active
    /// allies including the user, retaining side slot order.
    /// PRNG: none.
    pub fn allies_and_self(&self, user: MonId) -> Targets {
        self.active_side_targets(user.side(), false, None, None)
    }

    /// Ports sim/pokemon.ts:720-722. Returns living active allies except user.
    /// PRNG: none.
    pub fn allies(&self, user: MonId) -> Targets {
        self.active_side_targets(user.side(), false, Some(user), None)
    }

    /// Ports sim/pokemon.ts:724-726. Applies adjacency to living active allies.
    /// PRNG: none.
    pub fn adjacent_allies(&self, user: MonId) -> Targets {
        self.active_side_targets(user.side(), false, Some(user), Some(user))
    }

    /// Ports sim/pokemon.ts:728-730 and sim/side.ts:397-410. The all flag retains
    /// fainted active foes where Showdown's caller requests positional targets.
    /// PRNG: none.
    pub fn foes(&self, user: MonId, all: bool) -> Targets {
        self.active_side_targets(SideId(user.side().0 ^ 1), all, None, None)
    }

    /// Ports sim/pokemon.ts:732-735. Gen 9 doubles uses the living foe list.
    /// PRNG: none.
    pub fn adjacent_foes(&self, user: MonId) -> Targets {
        // In this fixed doubles format activePerHalf <= 2, so this source
        // path does not perform individual isAdjacent/fainted-user checks.
        self.foes(user, false)
    }

    /// Ports sim/battle-actions.ts:1544-1546. Tests CHOOSABLE_TARGETS for choices.
    /// PRNG: none.
    pub fn target_type_choices(&self, target_type: MoveTarget) -> bool {
        // adjacentAllyOrSelf/adjacentFoe are not represented by scoped moves.
        matches!(
            target_type,
            MoveTarget::Normal | MoveTarget::Any | MoveTarget::AdjacentAlly
        )
    }
}

#[cfg(test)]
#[path = "targetqueries/tests.rs"]
mod query_tests;
