//! Pinned Showdown state mutation ports.
#![allow(unused_variables, unused_imports)]
use crate::actions::mutators::common::{mon_arg, number};
use crate::{
    Battle,
    actions::*,
    dex::{self, ImmunityId, HookRel, DamageSpec, StatId},
    event::{EffectRef, EventArg, Relay, CallArgs, RunEventOptions},
    ids::*,
    log::{LogSink, LogArg, LogEntry},
    state::{
        CellId,
        scratch::{HitData, OrderedBoosts, ActiveMove, MoveEffectsScratch, move_runtime},
    },
};
impl<L: LogSink> Battle<L> {
    /// Damage number, false, null and undefined remain distinct
    /// Ports `sim/battle-actions.ts:1572-1709`. PRNG: critical roll at :1628 when modified ratio is truthy and willCrit absent; damage randomizer plus events.
    pub fn get_damage(
        &mut self,
        source: MonId,
        target: MonId,
        move_input: DamageInput,
        options: DamageOptions,
    ) -> Relay {
        match move_input {
            DamageInput::Move(MoveInput::Active(h)) => self.calculate_move_damage(source, target, h, options),
            DamageInput::Move(input) => { let h = self.get_active_move(input); let r = self.calculate_move_damage(source, target, h, options); self.release_active_move(h); r },
            DamageInput::BasePower(bp) => {
                let i = self.scratch.moves.iter().position(Option::is_none).expect("numeric move scratch exhausted") as u8;
                self.scratch.moves[i as usize] = Some(numeric_move(bp));
                let r = self.calculate_move_damage(source, target, MoveHandle(i), options);
                self.scratch.moves[i as usize] = None; r
            }
        }
    }
    /// Map damage independently over surviving spread targets
    /// Ports `sim/battle-actions.ts:1137-1168`. PRNG: get_damage for each Pokemon in source order.
    pub fn get_spread_damage(
        &mut self,
        targets: Targets,
        source: MonId,
        move_handle: MoveHandle,
    ) -> TargetResults {
        let mut initial = TargetResults { len: targets.len, ..TargetResults::default() };
        for i in 0..targets.len as usize { initial.values[i] = match targets.entries[i] { HitTarget::False => Relay::Bool(false), _ => Relay::Bool(true) }; }
        self.get_spread_damage_from(initial, targets, source, move_handle)
    }
    /// Exact spread/weather/crit/STAB/effectiveness/burn/modify/final clamp order
    /// Ports `sim/battle-actions.ts:1711-1835`. PRNG: one random(16) via battle.ts:2397 plus event sorts/callbacks.
    pub fn modify_damage(
        &mut self,
        base_damage: f64,
        source: MonId,
        target: MonId,
        move_handle: MoveHandle,
        options: DamageOptions,
    ) -> f64 {
        let e = EffectRef::ActiveMove(move_handle.0);
        if self.active_move(move_handle).move_type == TypeId::NONE { self.active_move_mut(move_handle).move_type = dex::type_id("???").unwrap(); }
        let m = *self.active_move(move_handle); let ty = m.move_type;
        let mut damage = base_damage + 2.;
        if m.runtime_flags & move_runtime::SPREAD_HIT != 0 || m.traits & dex::MOVE_TRAIT_SPREADHIT != 0 { damage = self.modify(damage, 0.75, 1.); }
        damage = number(self.priority_event(EventId::WeatherModifyDamage, mon_arg(Some(source)), mon_arg(Some(target)), e, Relay::Number(damage), false));
        let crit = self.move_hit_data(target, move_handle).crit;
        if crit { damage = tr(damage * 1.5); }
        damage = crate::math::randomizer(crate::math::trunc_f64(damage), self.state.prng.random(16)) as f64;
        if ty != dex::type_id("???").unwrap() {
            let is_stab = m.traits & dex::MOVE_TRAIT_FORCESTAB != 0 || self.has_type(source, &[ty]) || { let types = self.get_types(source, false, true); types.values[..types.len as usize].contains(&ty) };
            let mut stab = if is_stab { 1.5 } else { 1. };
            let tera = self.state.pokemon[source.0 as usize].terastallized;
            if tera == dex::type_id("Stellar").unwrap() {
                if self.state.pokemon[source.0 as usize].stellar_boosted_types & (1 << ty.0) == 0 || m.runtime_flags & move_runtime::STELLAR_BOOSTED != 0 {
                    stab = if is_stab { 2. } else { 4915. / 4096. };
                    self.active_move_mut(move_handle).runtime_flags |= move_runtime::STELLAR_BOOSTED;
                    if self.state.pokemon[source.0 as usize].species != dex::SPECIES_TERAPAGOSSTELLAR { self.state.pokemon[source.0 as usize].stellar_boosted_types |= 1 << ty.0; }
                }
            } else {
                // Preserve the second getTypes call used by the Tera STAB branch.
                if tera == ty { let types = self.get_types(source, false, true); if types.values[..types.len as usize].contains(&ty) { stab = 2.; } }
                stab = number(self.run_event(EventId::ModifySTAB, mon_arg(Some(source)), mon_arg(Some(target)), e, Relay::Number(stab), RunEventOptions::default()));
            }
            damage = self.modify(damage, stab, 1.);
        }
        let modifier = self.run_effectiveness(target, move_handle).clamp(-6, 6);
        self.move_hit_data(target, move_handle).type_mod = modifier;
        if modifier > 0 {
            if !options.suppress_messages { self.add(LogEntry::new("-supereffective", &[LogArg::Mon(target)], &[])); }
            for _ in 0..modifier { damage *= 2.; }
        } else if modifier < 0 {
            if !options.suppress_messages { self.add(LogEntry::new("-resisted", &[LogArg::Mon(target)], &[])); }
            for _ in 0..-modifier { damage = tr(damage / 2.); }
        }
        if crit && !options.suppress_messages { self.add(LogEntry::new("-crit", &[LogArg::Mon(target)], &[])); }
        if self.state.pokemon[source.0 as usize].status == crate::state::Status::Burn && m.category == dex::Category::Physical && !self.query_has_ability(source,"guts") && m.id != dex::MOVE_FACADE { damage = self.modify(damage, 0.5, 1.); }
        damage = number(self.run_event(EventId::ModifyDamage, mon_arg(Some(source)), mon_arg(Some(target)), e, Relay::Number(damage), RunEventOptions::default()));
        if self.move_hit_data(target, move_handle).bypass_protect { damage = self.modify(damage,0.25,1.); self.add(LogEntry::new("-zbroken", &[LogArg::Mon(target)], &[])); }
        if damage == 0. || damage.is_nan() { 1. } else { (crate::math::trunc_f64(damage) & 65535) as f64 }
    }
    /// Confusion uses its separate synthetic damage path
    /// Ports `sim/battle-actions.ts:1837-1855`. PRNG: one randomizer draw (battle.ts:2397); stat events.
    pub fn get_confusion_damage(&mut self, pokemon: MonId, base_power: f64) -> f64 {
        let attack = self.calculate_stat(pokemon, Stat::Atk, self.state.pokemon[pokemon.0 as usize].boosts[0], None, None);
        let defense = self.calculate_stat(pokemon, Stat::Def, self.state.pokemon[pokemon.0 as usize].boosts[1], None, None);
        let level = self.teams.sides[pokemon.side().0 as usize].sets[(pokemon.0 % 6) as usize].level as f64;
        let base = tr(tr(tr(tr(2. * level / 5. + 2.) * base_power * attack) / defense) / 50.) + 2.;
        let damage = crate::math::trunc_f64(base) & 65535;
        crate::math::randomizer(damage, self.state.prng.random(16)).max(1) as f64
    }
}

#[inline] fn tr(n: f64) -> f64 { crate::math::trunc_f64(n) as f64 }
fn stat(id: StatId) -> Stat { match id { StatId::Atk => Stat::Atk, StatId::Def => Stat::Def, StatId::SpA => Stat::SpA, StatId::SpD => Stat::SpD, StatId::Spe => Stat::Spe, _ => panic!("invalid damage stat") } }
fn stat_event(stat: Stat) -> EventId { match stat { Stat::Atk => EventId::ModifyAtk, Stat::Def => EventId::ModifyDef, Stat::SpA => EventId::ModifySpA, Stat::SpD => EventId::ModifySpD, Stat::Spe => EventId::ModifySpe } }
impl<L: LogSink> Battle<L> {
    /// Exact TS mapping with caller-owned incoming relays for falsy targets.
    /// Existing get_spread_damage is the primary-hit convenience wrapper.
    pub fn get_spread_damage_from(&mut self, mut results: TargetResults, targets: Targets, source: MonId, move_handle: MoveHandle) -> TargetResults {
        for i in 0..targets.len as usize {
            let HitTarget::Pokemon(t) = targets.entries[i] else { continue; };
            self.scratch.active_target = t;
            let r = self.get_damage(source, t, DamageInput::Move(MoveInput::Active(move_handle)), DamageOptions::default());
            // TS overwrites damage[i] with undefined before testing false, so its
            // inner '-fail' branch is unreachable. Both null and false become false.
            results.values[i] = if matches!(r, Relay::Null | Relay::Bool(false)) { Relay::Bool(false) } else { r };
        }
        results
    }
    fn calculate_move_damage(&mut self, source: MonId, target: MonId, h: MoveHandle, options: DamageOptions) -> Relay {
        let message = if options.suppress_messages { ImmunityMessage::Silent } else { ImmunityMessage::Standard };
        if !self.run_immunity(target, ImmunitySource::Move(h), message) { return Relay::Bool(false); }
        let e = EffectRef::ActiveMove(h.0); let m = *self.active_move(h);
        if let Some(hook) = self.event_hook(e, EventId::DamageCallback, HookRel::Direct) { return self.call_hook(hook, CallArgs { values: [mon_arg(Some(source)), mon_arg(Some(target)), EventArg::Undefined, EventArg::Undefined], len: 2 }); }
        let level = self.teams.sides[source.side().0 as usize].sets[(source.0 % 6) as usize].level as f64;
        match m.damage { DamageSpec::Level => return Relay::Number(level), DamageSpec::Fixed(n) if n != 0 => return Relay::Number(n as f64), _ => {} }
        let mut power = Relay::Number(m.base_power);
        let callback = self.event_hook(e, EventId::BasePowerCallback, HookRel::Direct);
        if let Some(hook) = callback { power = self.call_hook(hook, CallArgs { values: [mon_arg(Some(source)), mon_arg(Some(target)), EventArg::Effect(e), EventArg::Undefined], len: 3 }); }
        if !power.truthy() { return if power == Relay::Number(0.) { Relay::Undefined } else { power }; }
        let mut bp = number(power).floor().max(1.);
        let ratio = self.run_event(EventId::ModifyCritRatio, mon_arg(Some(source)), mon_arg(Some(target)), e, Relay::Number(m.crit_ratio as f64), RunEventOptions::default());
        let ratio = number(ratio).floor().clamp(0.,4.) as usize;
        let will_present = m.runtime_flags & move_runtime::WILL_CRIT_PRESENT != 0;
        let mut crit = if will_present { m.runtime_flags & move_runtime::WILL_CRIT != 0 } else { m.traits & dex::MOVE_TRAIT_WILLCRIT != 0 };
        if !will_present && m.traits & dex::MOVE_TRAIT_WILLCRIT == 0 && ratio != 0 { crit = self.state.prng.random_chance(1, [0,24,8,2,1][ratio]); }
        self.move_hit_data(target,h).crit = crit;
        if crit { crit = self.run_event(EventId::CriticalHit, mon_arg(Some(target)), EventArg::Null, e, Relay::Undefined, RunEventOptions::default()).truthy(); self.move_hit_data(target,h).crit = crit; }
        power = self.run_event(EventId::BasePower, mon_arg(Some(source)), mon_arg(Some(target)), e, Relay::Number(bp), RunEventOptions { on_effect: true, fast_exit: false });
        if !power.truthy() { return Relay::Number(0.); }
        bp = number(power).floor().max(1.);
        let tera = self.state.pokemon[source.0 as usize].terastallized;
        if tera != TypeId::NONE {
            let tera_power = if tera == dex::type_id("Stellar").unwrap() { self.state.pokemon[source.0 as usize].stellar_boosted_types & (1 << m.move_type.0) == 0 } else { self.has_type(source,&[m.move_type]) };
            let low_priority_single = m.id == EffectId::NONE || { let d = dex::move_data(m.id); d.priority <= 0 && d.multihit == [1,1] };
            if tera_power && bp < 60. && low_priority_single && !((m.base_power == 0. || m.base_power == 150.) && callback.is_some()) { bp = 60.; }
        }
        let attacker = if m.offensive_target { target } else { source }; let defender = target;
        let physical = m.category == dex::Category::Physical;
        let attack_stat = m.offensive_stat.map(stat).unwrap_or(if physical { Stat::Atk } else { Stat::SpA });
        let defense_stat = m.defensive_stat.map(stat).unwrap_or(if physical { Stat::Def } else { Stat::SpD });
        let mut atk_boost = self.state.pokemon[attacker.0 as usize].boosts[attack_stat as usize];
        let mut def_boost = self.state.pokemon[defender.0 as usize].boosts[defense_stat as usize];
        if m.traits & dex::MOVE_TRAIT_IGNOREOFFENSIVE != 0 || ((crit || m.traits & dex::MOVE_TRAIT_IGNORENEGATIVEOFFENSIVE != 0) && atk_boost < 0) { atk_boost = 0; }
        if m.traits & dex::MOVE_TRAIT_IGNOREDEFENSIVE != 0 || ((crit || m.traits & dex::MOVE_TRAIT_IGNOREPOSITIVEDEFENSIVE != 0) && def_boost > 0) { def_boost = 0; }
        let attack = self.calculate_stat(attacker,attack_stat,atk_boost,Some(1.),Some(source));
        let defense = self.calculate_stat(defender,defense_stat,def_boost,Some(1.),Some(target));
        let attack = number(self.run_event(if physical { EventId::ModifyAtk } else { EventId::ModifySpA }, mon_arg(Some(source)), mon_arg(Some(target)), e, Relay::Number(attack), RunEventOptions::default()));
        let defense = number(self.run_event(stat_event(defense_stat), mon_arg(Some(target)), mon_arg(Some(source)), e, Relay::Number(defense), RunEventOptions::default()));
        let base = tr(tr(tr(tr(2. * level / 5. + 2.) * bp * attack) / defense) / 50.);
        Relay::Number(self.modify_damage(base,source,target,h,options))
    }
}
fn numeric_move(bp: f64) -> ActiveMove {
    let effects = &dex::move_data(dex::MOVE_STRUGGLE).effects;
    ActiveMove {
        id: EffectId::NONE, flags: 0, traits: 0, runtime_flags: move_runtime::WILL_CRIT_PRESENT,
        total_damage: 0, hit_targets: [MonId::NONE;4], hit_target_len: 0, source_effect: EffectRef::None, type_changer_boosted: EffectId::NONE, ruined_stats: [MonId::NONE;4],
        base_power: bp, accuracy: crate::state::scratch::MoveAccuracy::Always, hit_data: [HitData::default();12], self_boosts: OrderedBoosts::default(),
        effects: MoveEffectsScratch { base: effects, boosts: OrderedBoosts::default(), status: EffectId::NONE, volatile_status: EffectId::NONE, side_condition: EffectId::NONE, slot_condition: EffectId::NONE, weather: EffectId::NONE, terrain: EffectId::NONE, pseudo_weather: EffectId::NONE, chance: None, heal: None, force_switch: false, self_switch: dex::SelfSwitch::None, suppressed_hooks: 0 },
        self_effect: None, secondaries: [None;4], secondary_count: 0, secondaries_present: false, recoil: None, drain: None, damage: DamageSpec::None, self_destruct: dex::SelfDestruct::None, offensive_stat: None, defensive_stat: None, offensive_target: false, ignore_immunity_types: 0, move_type: dex::type_id("???").unwrap(), category: dex::Category::Physical, target: dex::MoveTarget::Normal, priority: 0, crit_ratio: 1, hit: 0, multihit: [1,1],
    }
}
#[cfg(test)]
#[path = "calculation_tests.rs"]
mod tests;
