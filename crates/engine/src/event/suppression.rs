//! Pinned Showdown event ports; bodies supplied in stage 2B.
#![allow(unused_variables, unused_imports)]
use crate::{
    Battle,
    actions::{TargetResults, Targets},
    dex::{HookId, HookRel},
    event::*,
    ids::*,
    log::LogSink,
    state::{CellRef, EffectCell},
};
impl<L: LogSink> Battle<L> {
    /// Active move suppression differs from Pokemon.ignoringAbility.
    /// Ports `sim/battle.ts:365-368`. PRNG: none.
    pub fn suppressing_ability(&self, target: Option<MonId>) -> bool {
        todo!("stage 2B: suppressing_ability")
    }
    /// Independent singleEvent Start/End/TakeItem/SetAbility and weather exceptions.
    /// Ports `sim/battle.ts:605-628`. PRNG: none.
    pub fn single_event_suppressed(
        &self,
        event: EventId,
        effect: EffectRef,
        target: EventArg,
    ) -> bool {
        todo!("stage 2B: single_event_suppressed")
    }
    /// Do not prefilter during collection; re-evaluate immediately before dispatch.
    /// Ports `sim/battle.ts:824-839,873-919`. PRNG: none.
    pub fn run_event_suppressed(&self, event: EventId, listener: Listener) -> bool {
        todo!("stage 2B: run_event_suppressed")
    }
}
