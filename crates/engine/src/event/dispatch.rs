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
    /// Undefined relay defaults to true; null stays null. Captured removed state remains usable.
    /// Ports `sim/battle.ts:575-655`. PRNG: no direct draws; invoked handler may draw.
    pub fn single_event(
        &mut self,
        event: EventId,
        effect: EffectRef,
        state: Option<CellRef>,
        target: EventArg,
        source: EventArg,
        source_effect: EffectRef,
        relay: Relay,
        custom_hook: Option<HookId>,
    ) -> Relay {
        todo!("stage 2B: single_event")
    }
    /// Scalar event; null/undefined relay omit the callback relay argument. Restore parent frame.
    /// Ports `sim/battle.ts:762-944`. PRNG: selection-sort tie shuffles and callback draws.
    pub fn run_event(
        &mut self,
        event: EventId,
        target: EventArg,
        source: EventArg,
        source_effect: EffectRef,
        relay: Relay,
        options: RunEventOptions,
    ) -> Relay {
        todo!("stage 2B: run_event")
    }
    /// Array invocation retains per-target relay sentinels and callback indexes.
    /// Ports `sim/battle.ts:762-944`. PRNG: selection-sort tie shuffles and callback draws.
    pub fn run_event_spread(
        &mut self,
        event: EventId,
        targets: EventTargets,
        source: EventArg,
        source_effect: EffectRef,
        relays: TargetResults,
        options: RunEventOptions,
    ) -> TargetResults {
        todo!("stage 2B: run_event_spread")
    }
    /// First defined result, including false/null/zero. Uses fastExit redirect ordering.
    /// Ports `sim/battle.ts:947-952`. PRNG: stable redirect sort makes no draws; callbacks may draw.
    pub fn priority_event(
        &mut self,
        event: EventId,
        target: EventArg,
        source: EventArg,
        source_effect: EffectRef,
        relay: Relay,
        on_effect: bool,
    ) -> Relay {
        todo!("stage 2B: priority_event")
    }
    /// Run an event for each active Pokemon in exact sorted order.
    /// Ports `sim/battle.ts:469-483`. PRNG: active-speed tie shuffles then runEvent/callback draws.
    pub fn each_event(&mut self, event: EventId, effect: EffectRef, relay: Relay) -> () {
        todo!("stage 2B: each_event")
    }
    /// Field/Side suffixes, duration expiry and captured-cell location checks.
    /// Ports `sim/battle.ts:488-568`. PRNG: handler tie shuffles and callbacks.
    pub fn field_event(&mut self, event: EventId, targets: Option<EventTargets>) -> () {
        todo!("stage 2B: field_event")
    }
    /// Direct callback: preserves current event/effect/state and does not increment eventDepth.
    /// Ports `sim/battle-actions.ts:1600,1614; sim/pokemon.ts:2010; sim/battle.ts:1945`. PRNG: callback only; no discovery sort.
    pub fn call_hook(&mut self, hook: HookId, args: CallArgs) -> Relay {
        todo!("stage 2B: call_hook")
    }
    /// Read exact callback argument order; direct callbacks have a separate CallArgs slot.
    /// Ports `sim/battle.ts:641,845-854`. PRNG: none.
    pub fn event_arg(&self, cx: HookCtx, index: usize) -> EventArg {
        todo!("stage 2B: event_arg")
    }
    /// Read the captured object, including retired pinned state; do not use arena.is_live as a guard.
    /// Ports `sim/battle.ts:634,841`. PRNG: none.
    pub fn hook_state(&self, cx: HookCtx) -> &EffectCell {
        todo!("stage 2B: hook_state")
    }
    /// Mutate captured effectState. Payload presence remains effect-owned.
    /// Ports `sim/battle.ts:634,841`. PRNG: none.
    pub fn hook_state_mut(&mut self, cx: HookCtx) -> &mut EffectCell {
        todo!("stage 2B: hook_state_mut")
    }
    /// Frame 255 denotes the initial empty event context.
    /// Ports `sim/battle.ts:636,842`. PRNG: none.
    pub fn event_frame(&self, cx: HookCtx) -> Option<&EventFrame> {
        todo!("stage 2B: event_frame")
    }
    /// Direct calls reuse the parent owner/state; scratch stores current context.
    /// Ports `sim/battle.ts:633-636,840-842`. PRNG: none.
    pub fn current_hook_context(&self) -> HookCtx {
        todo!("stage 2B: current_hook_context")
    }
    /// Depth >=8 fails; singleEvent also guards >1000 unsent logical entries.
    /// Ports `sim/battle.ts:582-592,772-775`. PRNG: none.
    pub fn check_event_limits(&mut self, event: EventId, single: bool) -> () {
        todo!("stage 2B: check_event_limits")
    }
}
