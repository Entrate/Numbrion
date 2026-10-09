//! Gen 9 doubles location, target, and redirection rules. OWNER M.
#![allow(unused_variables)]
use crate::{
    actions::{MoveHandle, MoveInput, MoveTargets, Targets},
    battle::Battle,
    dex::MoveTarget,
    ids::{MonId, SlotId},
    log::LogSink,
};

impl<L: LogSink> Battle<L> {
    /// Ports sim/battle.ts:2403-2436. Checks signed locations against move target
    /// kind without resolving an occupant; used by the text validator.
    /// PRNG: none.
    pub fn valid_target_loc(&self, target_loc: i8, source: MonId, target_type: MoveTarget) -> bool {
        todo!("stage M: target location legality")
    }

    /// Ports sim/battle.ts:2437-2439. Location-based legality for a concrete mon.
    /// PRNG: none.
    pub fn valid_target(&self, target: MonId, source: MonId, target_type: MoveTarget) -> bool {
        todo!("stage M: concrete target legality")
    }

    /// Ports sim/battle.ts:2441-2492. Tracks original targets, validates selected
    /// positions, preserves fainted ally targets, and applies fallback targeting.
    /// PRNG: fallback get_random_target samples; no draw for a valid selected target.
    pub fn get_target(
        &mut self,
        user: MonId,
        move_input: MoveInput,
        target_loc: Option<i8>,
        original_target: Option<MonId>,
    ) -> Option<MonId> {
        todo!("stage M: resolve chosen target")
    }

    /// Ports sim/battle.ts:2494-2530 and sim/side.ts:365-372. Automatic targeting
    /// samples eligible foes/allies in array order, including singleton samples.
    /// PRNG: exactly one sample draw when a nonempty candidate list is sampled;
    /// self/side/field target kinds and empty-list fallback make no draw.
    pub fn get_random_target(&mut self, user: MonId, move_input: MoveInput) -> Option<MonId> {
        todo!("stage M: random target selection")
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
        todo!("stage M: spread and Pressure targets")
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
        todo!("stage M: move redirection")
    }

    /// Ports sim/pokemon.ts:770-782. Resolves a signed relative active position.
    /// PRNG: none.
    pub fn get_at_loc(&self, user: MonId, target_loc: i8) -> Option<MonId> {
        todo!("stage M: location occupant")
    }

    /// Ports sim/pokemon.ts:784-789. Converts a current position to signed loc.
    /// PRNG: none.
    pub fn get_loc_of(&self, user: MonId, target: MonId) -> i8 {
        todo!("stage M: relative target location")
    }

    /// Ports sim/pokemon.ts:520-527. The slot follows current party position;
    /// its encoding differs from stable MonId and cross-side field rank.
    /// PRNG: none.
    pub fn get_slot(&self, mon: MonId) -> SlotId {
        todo!("stage M: current positional slot")
    }

    /// Ports sim/pokemon.ts:737-739. Checks side/team affiliation.
    /// PRNG: none.
    pub fn is_ally(&self, mon: MonId, other: Option<MonId>) -> bool {
        todo!("stage M: ally predicate")
    }

    /// Ports sim/pokemon.ts:741-747. Gen 9 doubles positional adjacency.
    /// PRNG: none.
    pub fn is_adjacent(&self, mon: MonId, other: MonId) -> bool {
        todo!("stage M: adjacency predicate")
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
        todo!("stage M: Dragon Darts smart targets")
    }

    /// Ports sim/pokemon.ts:716-718 and sim/side.ts:390-395. Returns living active
    /// allies including the user, retaining side slot order.
    /// PRNG: none.
    pub fn allies_and_self(&self, user: MonId) -> Targets {
        todo!("stage M: allies and self")
    }

    /// Ports sim/pokemon.ts:720-722. Returns living active allies except user.
    /// PRNG: none.
    pub fn allies(&self, user: MonId) -> Targets {
        todo!("stage M: allies")
    }

    /// Ports sim/pokemon.ts:724-726. Applies adjacency to living active allies.
    /// PRNG: none.
    pub fn adjacent_allies(&self, user: MonId) -> Targets {
        todo!("stage M: adjacent allies")
    }

    /// Ports sim/pokemon.ts:728-730 and sim/side.ts:397-410. The all flag retains
    /// fainted active foes where Showdown's caller requests positional targets.
    /// PRNG: none.
    pub fn foes(&self, user: MonId, all: bool) -> Targets {
        todo!("stage M: foes")
    }

    /// Ports sim/pokemon.ts:732-735. Gen 9 doubles uses the living foe list.
    /// PRNG: none.
    pub fn adjacent_foes(&self, user: MonId) -> Targets {
        todo!("stage M: adjacent foes")
    }

    /// Ports sim/battle-actions.ts:1544-1546. Tests CHOOSABLE_TARGETS for choices.
    /// PRNG: none.
    pub fn target_type_choices(&self, target_type: MoveTarget) -> bool {
        todo!("stage M: target kind requires user selection")
    }
}
