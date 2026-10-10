//! Declarative primary/self/secondary move effects. OWNER M.
use super::support::*;
use crate::{
    actions::{
        Attribution, HealEffect, HitEffect, HitOptions, HitTarget, MoveHandle, TargetResults,
        Targets,
    },
    battle::Battle,
    dex::{self, SelfDestruct, SelfSwitch},
    event::{EffectRef, EventArg, Relay, RunEventOptions},
    ids::{EventId, Holder, MonId},
    log::{LogArg, LogEntry, LogSink, MoveLineEdit},
    state::scratch::{SecondaryScratch, move_runtime as rt},
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
        let mut damage = damage;
        let (is_secondary, is_self) = (options.secondary, options.self_hit);
        let effect = EffectRef::ActiveMove(move_handle.0);
        let attribution = Attribution::from_move(user, effect);
        // `damage.reduce(combineResults)` (a reduce over an empty array throws).
        assert!(
            damage.count() > 0,
            "runMoveEffects over an empty damage array"
        );
        let mut did_anything = damage.at(0);
        for i in 1..damage.count() {
            did_anything = combine_results(did_anything, damage.at(i));
        }
        for i in 0..targets.count() {
            let slot = targets.at(i);
            if slot == HitTarget::False {
                continue;
            }
            let view = self.hit_view(move_handle, hit_effect);
            let mut did_something = Relay::Undefined;
            if let HitTarget::Pokemon(target) = slot {
                if view.effects.boosts.len > 0 && !self.mon_fainted(target) {
                    let hit_result = self.boost(
                        view.effects.boosts,
                        Some(target),
                        attribution,
                        is_secondary,
                        is_self,
                    );
                    did_something = combine_results(did_something, hit_result);
                }
                if let Some(heal) = view.effects.heal {
                    if !self.mon_fainted(target) {
                        let (hp, max_hp) = {
                            let p = &self.state.pokemon[target.0 as usize];
                            (p.hp, p.max_hp)
                        };
                        if hp >= max_hp {
                            self.add(LogEntry::new(
                                "-fail",
                                &[LogArg::Mon(target), LogArg::Text("heal")],
                                &[],
                            ));
                            self.attr_last_move(MoveLineEdit::Still);
                            damage.set(i, combine_results(damage.at(i), Relay::FAIL));
                            did_anything = combine_results(did_anything, Relay::Null);
                            continue;
                        }
                        // baseMaxhp aliases maxhp in this format (no Dynamax).
                        let amount = f64::from(max_hp) * f64::from(heal[0]) / f64::from(heal[1]);
                        let d = self.heal(
                            js_round(amount),
                            Some(target),
                            Some(user),
                            HealEffect::Effect(effect),
                        );
                        if failed_not_zero(d) {
                            if d != Relay::Null {
                                self.add(LogEntry::new("-fail", &[LogArg::Mon(user)], &[]));
                                self.attr_last_move(MoveLineEdit::Still);
                            }
                            damage.set(i, combine_results(damage.at(i), Relay::FAIL));
                            did_anything = combine_results(did_anything, Relay::Null);
                            continue;
                        }
                        did_something = Relay::Bool(true);
                    }
                }
                if view.effects.status != crate::ids::EffectId::NONE {
                    // `moveData.ability` is never set by a scoped effect; the source is the move.
                    let hit_result = self.try_set_status(target, view.effects.status, attribution);
                    if !hit_result.truthy()
                        && self.active_move(move_handle).effects.status
                            != crate::ids::EffectId::NONE
                    {
                        damage.set(i, combine_results(damage.at(i), Relay::FAIL));
                        did_anything = combine_results(did_anything, Relay::Null);
                        continue;
                    }
                    did_something = combine_results(did_something, hit_result);
                }
                // `moveData.forceStatus` exists on no scoped move or secondary.
                if view.effects.volatile_status != crate::ids::EffectId::NONE {
                    let hit_result =
                        self.add_volatile(target, view.effects.volatile_status, attribution, None);
                    did_something = combine_results(did_something, hit_result);
                }
                if view.effects.side_condition != crate::ids::EffectId::NONE {
                    let hit_result = self.add_side_condition(
                        target.side(),
                        view.effects.side_condition,
                        attribution,
                    );
                    did_something = combine_results(did_something, hit_result);
                }
                if view.effects.slot_condition != crate::ids::EffectId::NONE {
                    let hit_result = self.add_slot_condition(
                        self.get_slot(target),
                        view.effects.slot_condition,
                        attribution,
                    );
                    did_something = combine_results(did_something, hit_result);
                }
                if view.effects.weather != crate::ids::EffectId::NONE {
                    let hit_result = self.set_weather(view.effects.weather, attribution);
                    did_something = combine_results(did_something, hit_result);
                }
                if view.effects.terrain != crate::ids::EffectId::NONE {
                    let hit_result = self.set_terrain(view.effects.terrain, attribution);
                    did_something = combine_results(did_something, Relay::Bool(hit_result));
                }
                if view.effects.pseudo_weather != crate::ids::EffectId::NONE {
                    let hit_result =
                        self.add_pseudo_weather(view.effects.pseudo_weather, attribution);
                    did_something = combine_results(did_something, hit_result);
                }
                if view.effects.force_switch {
                    let hit_result = self.can_switch(target.side()) != 0;
                    did_something = combine_results(did_something, Relay::Bool(hit_result));
                }
                // Hit events: like the TryHit events, except there is no FieldHit event.
                let kind = self.active_move(move_handle).target;
                if kind == dex::MoveTarget::All && !is_self {
                    if let Some(hook) = self.hit_hook(&view, EventId::HitField) {
                        let hit_result = self.single_event(
                            EventId::HitField,
                            view.callback_effect,
                            None,
                            EventArg::Holder(Holder::mon(target)),
                            EventArg::Holder(Holder::mon(user)),
                            effect,
                            Relay::Undefined,
                            Some(hook),
                        );
                        did_something = combine_results(did_something, hit_result);
                    }
                } else if matches!(kind, dex::MoveTarget::FoeSide | dex::MoveTarget::AllySide)
                    && !is_self
                {
                    if let Some(hook) = self.hit_hook(&view, EventId::HitSide) {
                        let hit_result = self.single_event(
                            EventId::HitSide,
                            view.callback_effect,
                            None,
                            EventArg::Holder(Holder::side(target.side())),
                            EventArg::Holder(Holder::mon(user)),
                            effect,
                            Relay::Undefined,
                            Some(hook),
                        );
                        did_something = combine_results(did_something, hit_result);
                    }
                } else {
                    if let Some(hook) = self.hit_hook(&view, EventId::Hit) {
                        let hit_result = self.single_event(
                            EventId::Hit,
                            view.callback_effect,
                            None,
                            EventArg::Holder(Holder::mon(target)),
                            EventArg::Holder(Holder::mon(user)),
                            effect,
                            Relay::Undefined,
                            Some(hook),
                        );
                        did_something = combine_results(did_something, hit_result);
                    }
                    if !is_self && !is_secondary {
                        self.run_event(
                            EventId::Hit,
                            EventArg::Holder(Holder::mon(target)),
                            EventArg::Holder(Holder::mon(user)),
                            effect,
                            Relay::Undefined,
                            RunEventOptions::default(),
                        );
                    }
                }
            }
            // Re-read: callbacks above may have edited the move's own fields.
            let view = self.hit_view(move_handle, hit_effect);
            if view.self_destruct == SelfDestruct::IfHit && !is_false(damage.at(i)) {
                self.faint(user, attribution);
            }
            if view.effects.self_switch != SelfSwitch::None {
                // `source.volatiles['commanded']` cannot exist: Commander is out of scope.
                if self.can_switch(user.side()) != 0 {
                    did_something = Relay::Bool(true);
                } else {
                    did_something = combine_results(did_something, Relay::FAIL);
                }
            }
            // Move didn't fail because it didn't try to do anything.
            if did_something == Relay::Undefined {
                did_something = Relay::Bool(true);
            }
            let folded = if did_something == Relay::Null {
                Relay::FAIL
            } else {
                did_something
            };
            damage.set(i, combine_results(damage.at(i), folded));
            did_anything = combine_results(did_anything, did_something);
        }
        let view = self.hit_view(move_handle, hit_effect);
        if failed_not_zero(did_anything)
            && !view.has_self
            && view.self_destruct == SelfDestruct::None
        {
            if !is_self && !is_secondary && is_false(did_anything) {
                self.add(LogEntry::new("-fail", &[LogArg::Mon(user)], &[]));
                self.attr_last_move(MoveLineEdit::Still);
            }
        } else if self.active_move(move_handle).effects.self_switch != SelfSwitch::None
            && self.mon_hp(user) != 0
        {
            // U-turn, Volt Switch, Flip Turn, Parting Shot, Baton Pass, Shed Tail, ...
            let id = self.active_move(move_handle).id;
            self.state.pokemon[user.0 as usize].switch_flag = id;
        }
        damage
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
        for i in 0..targets.count() {
            if targets.at(i) == HitTarget::False {
                continue;
            }
            let view = self.hit_view(move_handle, hit_effect);
            if view.has_self && !self.mflag(move_handle, rt::SELF_DROPPED) {
                let self_effect = view.self_effect.expect("moveData.self");
                let drop = match hit_effect {
                    HitEffect::Secondary(code) => HitEffect::SecondarySelf(code),
                    HitEffect::Primary => HitEffect::SelfEffect,
                    other => panic!("a {other:?} hit effect has no self object"),
                };
                if !secondary && self_effect.boosts.len > 0 {
                    let roll = self.state.prng.random(100);
                    if self_effect.chance.is_none_or(|c| roll < u32::from(c)) {
                        self.move_hit(
                            Targets::from_mons(&[user]),
                            user,
                            move_handle,
                            drop,
                            HitOptions {
                                secondary,
                                self_hit: true,
                            },
                        );
                    }
                    if !self.mflag(move_handle, rt::MULTIHIT_PRESENT) {
                        self.set_mflag(move_handle, rt::SELF_DROPPED, true);
                    }
                } else {
                    self.move_hit(
                        Targets::from_mons(&[user]),
                        user,
                        move_handle,
                        drop,
                        HitOptions {
                            secondary,
                            self_hit: true,
                        },
                    );
                }
            }
        }
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
        let view = self.hit_view(move_handle, hit_effect);
        if !view.has_secondaries {
            return;
        }
        for i in 0..targets.count() {
            let slot = targets.at(i);
            if slot == HitTarget::False {
                continue;
            }
            // `moveData.secondaries.slice()` as a mutable relay array.
            let list = self.active_move(move_handle).secondaries;
            let original = self.stash_secondaries(list);
            let target_arg = match slot {
                HitTarget::Pokemon(m) => mon_arg(m),
                _ => EventArg::Null,
            };
            let result = self.run_event(
                EventId::ModifySecondaries,
                target_arg,
                mon_arg(user),
                EffectRef::ActiveMove(move_handle.0),
                Relay::Secondaries(original),
                RunEventOptions::default(),
            );
            let Relay::Secondaries(handle) = result else {
                panic!("ModifySecondaries returned a non-array relay {result:?}");
            };
            let secondaries: [Option<SecondaryScratch>; 4] = *self.scratch_secondaries(handle);
            for (index, entry) in secondaries.iter().enumerate() {
                let Some(secondary) = entry else {
                    continue;
                };
                // User stat boosts or target stat drops overflow only in Gen 8 or prior.
                let roll = self.state.prng.random(100);
                if !secondary.chance_present || roll < u32::from(secondary.chance) {
                    self.move_hit(
                        match slot {
                            HitTarget::Pokemon(m) => Targets::from_mons(&[m]),
                            other => Targets::single(other),
                        },
                        user,
                        move_handle,
                        HitEffect::Secondary(secondary_code(handle, index)),
                        HitOptions {
                            secondary: true,
                            self_hit,
                        },
                    );
                }
            }
            self.release_relay(result);
            if handle != original {
                self.release_relay(Relay::Secondaries(original));
            }
        }
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
        let mut damage = damage;
        for i in 0..targets.count() {
            let Some(target) = targets.mon_at(i) else {
                continue;
            };
            if self.mon_hp(target) > 0
                && self.mon_hp(user) > 0
                && self.can_switch(target.side()) != 0
            {
                let hit_result = self.run_event(
                    EventId::DragOut,
                    mon_arg(target),
                    mon_arg(user),
                    EffectRef::ActiveMove(move_handle.0),
                    Relay::Undefined,
                    RunEventOptions::default(),
                );
                if hit_result.truthy() {
                    self.state.pokemon[target.0 as usize].flags |=
                        crate::state::mon_flags::FORCE_SWITCH;
                } else if is_false(hit_result)
                    && self.active_move(move_handle).category == dex::Category::Status
                {
                    self.add(LogEntry::new("-fail", &[LogArg::Mon(user)], &[]));
                    self.attr_last_move(MoveLineEdit::Still);
                    damage.set(i, Relay::FAIL);
                }
            }
        }
        damage
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
        let (traits, recoil) = {
            let m = self.active_move(move_handle);
            (m.traits, m.recoil)
        };
        let struggle_recoil = traits & dex::MOVE_TRAIT_STRUGGLERECOIL != 0;
        let max_hp = f64::from(self.state.pokemon[user.0 as usize].max_hp);
        // mindBlownRecoil and chloroblastRecoil exist on no scoped move.
        let recoil_damage = if struggle_recoil {
            clamp_int_range(js_round(max_hp / 4.0), Some(1.0), None)
        } else if let Some(r) = recoil {
            clamp_int_range(
                js_round(damage_dealt * f64::from(r[0]) / f64::from(r[1])),
                Some(1.0),
                None,
            )
        } else {
            return Relay::Null;
        };
        let hp_before = self.mon_hp(user);
        if struggle_recoil {
            self.direct_damage(
                recoil_damage,
                Some(user),
                Attribution::from_move(
                    user,
                    EffectRef::Synthetic(crate::event::SyntheticEffect::StruggleRecoil),
                ),
            );
        } else {
            self.damage(
                recoil_damage,
                Some(user),
                Attribution::from_move(
                    user,
                    EffectRef::Synthetic(crate::event::SyntheticEffect::Recoil),
                ),
            );
        }
        self.run_event(
            EventId::EmergencyExit,
            mon_arg(user),
            mon_arg(user),
            EffectRef::None,
            Relay::Number(f64::from(hp_before)),
            RunEventOptions::default(),
        );
        Relay::Number(recoil_damage)
    }

    /// Ports sim/battle-actions.ts:1548-1564 and result reduction at1179-1187.
    /// Keeps numerical zero distinct from failure and undefined; shared by
    /// declarative effects when combining callback and damage results.
    /// PRNG: none.
    pub fn combine_move_results(&self, previous: Relay, next: Relay) -> Relay {
        combine_results(previous, next)
    }
}
