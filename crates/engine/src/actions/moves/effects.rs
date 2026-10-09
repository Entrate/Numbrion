//! Declarative primary/self/secondary move effects. OWNER M.
#![allow(unused_variables)]
use crate::{
    actions::{HitEffect, HitOptions, MoveHandle, TargetResults, Targets},
    battle::Battle,
    event::Relay,
    ids::MonId,
    log::LogSink,
};

impl<L: LogSink> Battle<L> {
    /// Ports sim/battle-actions.ts:1175-1305. Applies boosts, healing, status,
    /// volatile/side/slot/field effects, Hit events, selfdestruct, and selfSwitch.
    /// PRNG: none directly; core mutation callbacks and hit events may draw.
    pub fn run_move_effects(
        &mut self,
        damage: TargetResults,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
        hit_effect: HitEffect,
        options: HitOptions,
    ) -> TargetResults {
        todo!("stage M: declarative move effects")
    }

    /// Ports sim/battle-actions.ts:1306-1324. Null targets remain eligible;
    /// preserves selfDropped handling across hits and secondary invocation.
    /// PRNG: random(100) at line 1314 for non-secondary self.boosts even when
    /// chance is absent; nested move_hit/events may draw additionally.
    pub fn self_drops(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
        hit_effect: HitEffect,
        secondary: bool,
    ) {
        todo!("stage M: declarative self effects")
    }

    /// Ports sim/battle-actions.ts:1325-1341. Copies/modifies secondaries per
    /// surviving target, then executes them in array order.
    /// PRNG: random(100) for every secondary at line 1332, including absent or
    /// 100% chance; ModifySecondaries and nested hit callbacks may also draw.
    pub fn secondaries(
        &mut self,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
        hit_effect: HitEffect,
        self_hit: bool,
    ) {
        todo!("stage M: secondary effects")
    }

    /// Ports sim/battle-actions.ts:1342-1358. Sets forceSwitchFlag after DragOut;
    /// lifecycle later performs the random drag and slot transition.
    /// PRNG: none directly; DragOut callbacks/order may draw. Drag selection is
    /// deferred to lifecycle and must not draw here.
    pub fn force_switch(
        &mut self,
        damage: TargetResults,
        targets: Targets,
        user: MonId,
        move_handle: MoveHandle,
    ) -> TargetResults {
        todo!("stage M: forced-switch marking")
    }

    /// Ports sim/battle-actions.ts:1368-1387. Rounds ordinary/Struggle/half-HP
    /// recoil before D's damage or direct_damage; returns null if no recoil.
    /// PRNG: none directly; ordinary damage events may draw.
    pub fn apply_recoil_damage(
        &mut self,
        damage_dealt: f64,
        move_handle: MoveHandle,
        user: MonId,
    ) -> Relay {
        todo!("stage M: recoil calculation and application")
    }

    /// Ports sim/battle-actions.ts:1548-1564 and result reduction at1179-1187.
    /// Keeps numerical zero distinct from failure and undefined; shared by
    /// declarative effects when combining callback and damage results.
    /// PRNG: none.
    pub fn combine_move_results(&self, previous: Relay, next: Relay) -> Relay {
        todo!("stage M: move result combination")
    }
}
