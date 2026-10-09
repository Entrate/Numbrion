//! Start, commit, action dispatch and turn boundaries. OWNER L.
#![allow(unused_variables)]

use crate::sim::BattleError;
use crate::{Battle, event::Relay, ids::MonId, log::LogSink, state::choices::Action};

/// HP captures used after the residual action; battle.ts:2671,2819,2860-2863.
/// PRNG: capture draws nothing; EmergencyExit events may draw afterwards.
#[derive(Clone, Copy, Debug)]
pub struct ResidualSnapshot {
    pub entries: [(MonId, u16); 4],
    pub len: u8,
}

impl Default for ResidualSnapshot {
    fn default() -> Self {
        Self {
            entries: [(MonId::NONE, 0); 4],
            len: 0,
        }
    }
}

impl<L: LogSink> Battle<L> {
    /// Start synchronously through the first request; sim/battle.ts:1909-1973.
    /// PRNG: Begin/BattleStart callbacks, initial switch/runSwitch/event ordering.
    pub fn start(&mut self) -> Result<(), BattleError> {
        todo!("L: start to the first decision boundary")
    }

    /// Execute the start action; sim/battle.ts:2674-2707.
    /// PRNG: BattleStart events, switchIn/runSwitch ties and switch-in callbacks.
    pub fn run_start_action(&mut self) {
        todo!("L: start action and initial switches")
    }

    /// Pick-team callbacks; sim/battle.ts:1981-2011.
    /// This format has no team preview; preserve direct callback context.
    /// PRNG: callbacks only; scoped format/rules currently draw none here.
    pub fn run_pick_team(&mut self) {
        todo!("L: runPickTeam for the pinned format")
    }

    /// Resolve choices before saved midturn queue; sim/battle.ts:2996-3028.
    /// PRNG: updateSpeed, p1 then p2 resolution, sort only new queue, resumed loop.
    pub fn commit_choices(&mut self) -> Result<(), BattleError> {
        todo!("L: commitChoices, preserving the unsorted saved queue")
    }

    /// Run until request/end, otherwise endTurn; sim/battle.ts:2936-2955.
    /// PRNG: beforeTurn insertion, action callbacks, residuals and endTurn events.
    pub fn turn_loop(&mut self) {
        todo!("L: turnLoop")
    }

    /// Execute each scoped action and shared postprocessing; battle.ts:2669-2933.
    /// PRNG: delegated action/events, faint processing, forced switches, dynamic queue sort.
    /// True means the action ended in a request or battle end.
    pub fn run_action(&mut self, action: Action) -> bool {
        todo!("L: runAction for every scoped ActionKind")
    }

    /// Residual action's clearActiveMove/updateSpeed/fieldEvent sequence;
    /// sim/battle.ts:2815-2822. PRNG: event ordering/tie shuffles and callbacks.
    /// Capture original HP per active before residuals for EmergencyExit.
    pub fn run_residual(&mut self) -> ResidualSnapshot {
        todo!("L: residual action, including original-HP captures")
    }

    /// Advance turn and rebuild trapping/disabling/requests; battle.ts:1627-1805.
    /// PRNG: DisableMove, TrapPokemon/MaybeTrapPokemon and hypothetical ability events.
    /// Gen2/3 Quick Claw draws are absent from this Gen9 path.
    pub fn end_turn(&mut self) {
        todo!("L: endTurn and next move request")
    }

    /// Enforce warnings and auto-tie after turn 1000; battle.ts:1807-1907.
    /// PRNG: none; Endless Battle Clause itself is not enabled in this format.
    pub fn check_turn_limit(&mut self) -> bool {
        todo!("L: maybeTriggerEndlessBattleClause's enabled turn-limit path")
    }

    /// Update every non-fainted active's cached speed; battle.ts:391-395.
    /// PRNG: getStat/getActionSpeed event sorting and handlers only.
    pub fn update_speed(&mut self) {
        todo!("L: updateSpeed")
    }

    /// Post-action pending forced switches and request boundary; battle.ts:2825-2933.
    /// PRNG: dragIn sampling, Update/EmergencyExit/BeforeSwitchOut events and queue sort.
    pub fn process_action_boundary(
        &mut self,
        action: Action,
        original_hp: Option<u16>,
        residual: ResidualSnapshot,
    ) -> bool {
        todo!("L: common runAction postprocessing")
    }

    /// Revival action revives at half HP and can queue instaswitch;
    /// battle.ts:2785-2802. PRNG: queued instaswitch resolution/events; no direct draw.
    pub fn run_revival_blessing(&mut self, user: MonId, target: MonId) {
        todo!("L: Revival Blessing action")
    }

    /// BeforeTurnMove action wrapper; battle.ts:2734-2742.
    /// Delegate active/fainted checks and direct callback to M's run_before_turn_move.
    /// PRNG: getTarget may sample, then direct callback; preserve parent event/effect frame.
    pub fn run_before_turn_move_action(&mut self, action: Action) -> Relay {
        todo!("L: beforeTurnMove action delegates to M")
    }

    /// PriorityChargeMove action wrapper; battle.ts:2743-2749.
    /// Delegate callback execution to M's run_priority_charge_move.
    /// PRNG: direct callback only; preserve the current event/effect frame.
    pub fn run_priority_charge_move_action(&mut self, action: Action) -> Relay {
        todo!("L: priorityChargeMove action delegates to M")
    }
}
