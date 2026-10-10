//! Pinned Showdown state mutation ports.
#![allow(unused_variables, unused_imports)]
use super::common::{mon_arg, number};
use crate::{
    Battle,
    actions::*,
    dex::{self, ImmunityId},
    event::{EffectRef, Relay, RunEventOptions},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{
        CellId, mon_flags,
        scratch::SyntheticEffect,
        scratch::{HitData, OrderedBoosts},
    },
};
impl<L: LogSink> Battle<L> {
    /// Delegates through spread_damage; preserve source defaulting and sentinels
    /// Ports `sim/battle.ts:2199-2209`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn damage(
        &mut self,
        amount: f64,
        target: Option<MonId>,
        attribution: Attribution,
    ) -> Relay {
        let a = self.mutation_attribution(attribution, false, true);
        let target = target.or_else(|| self.mutation_frame().and_then(|f| Self::arg_mon(f.target)));
        self.spread_damage(
            TargetResults {
                values: [
                    Relay::Number(amount),
                    Relay::Undefined,
                    Relay::Undefined,
                    Relay::Undefined,
                ],
                len: 1,
            },
            Targets {
                entries: [
                    target.map_or(HitTarget::Null, HitTarget::Pokemon),
                    HitTarget::False,
                    HitTarget::False,
                    HitTarget::False,
                ],
                len: 1,
            },
            a,
            false,
        )
        .values[0]
    }
    /// Apply Damage then mutate/log/faint/drain/AfterDamage in source order
    /// Ports `sim/battle.ts:2095-2197`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn spread_damage(
        &mut self,
        amounts: TargetResults,
        targets: Targets,
        attribution: Attribution,
        instafaint: bool,
    ) -> TargetResults {
        let mut result = TargetResults {
            len: amounts.len,
            ..TargetResults::default()
        };
        for i in 0..amounts.len as usize {
            let cur = amounts.values[i];
            if !cur.truthy() && cur != Relay::Number(0.) {
                result.values[i] = cur;
                continue;
            }
            let HitTarget::Pokemon(t) = targets.entries[i] else {
                result.values[i] = Relay::Number(0.);
                continue;
            };
            let p = &self.state.pokemon[t.0 as usize];
            if p.hp == 0 {
                result.values[i] = Relay::Number(0.);
                continue;
            }
            if p.flags & mon_flags::ACTIVE == 0 {
                result.values[i] = Relay::Bool(false);
                continue;
            }
            let e = attribution.effect;
            let id = self.event_effect_id(e);
            let kind = self.event_effect_type(e);
            let mut d = if cur == Relay::Number(0.) {
                cur
            } else {
                Relay::Number(number(cur).floor().max(1.))
            };
            if e != EffectRef::Synthetic(SyntheticEffect::StruggleRecoil) {
                if kind == dex::EffectType::Weather {
                    let immunity = match id {
                        dex::CONDITION_SANDSTORM => Some(ImmunityId::Sandstorm),
                        _ => None,
                    };
                    if immunity
                        .is_some_and(|im| !self.run_status_immunity(t, im, ImmunityMessage::Silent))
                    {
                        result.values[i] = Relay::Number(0.);
                        continue;
                    }
                }
                d = self.run_event(
                    EventId::Damage,
                    mon_arg(Some(t)),
                    attribution.source,
                    e,
                    d,
                    RunEventOptions {
                        on_effect: true,
                        fast_exit: false,
                    },
                );
                if !d.truthy() && d != Relay::Number(0.) {
                    result.values[i] = if cur == Relay::Bool(true) {
                        Relay::Undefined
                    } else {
                        d
                    };
                    continue;
                }
            }
            let n = if d == Relay::Number(0.) {
                0.
            } else {
                number(d).floor().max(1.)
            };
            let actual = self.raw_damage(t, n, attribution);
            result.values[i] = Relay::Number(actual);
            let source = Self::arg_mon(attribution.source);
            let mut tags = [LogTag::Bare("silent"); 2];
            let len;
            if id == dex::CONDITION_PARTIALLYTRAPPED {
                let cell = self.get_volatile(t, id).expect("missing partial trap");
                tags[0] = LogTag::From(
                    EffectToken(self.state.effects.cells[cell.0 as usize].source_effect.0)
                        .resolve(),
                );
                tags[1] = LogTag::Bare("partiallytrapped");
                len = 2;
            } else if id != EffectId::NONE && dex::effect(id).key == "powder" {
                len = 1;
            } else if e == EffectRef::Synthetic(SyntheticEffect::Confused) {
                tags[0] = LogTag::Value("from", LogArg::Text("confusion"));
                len = 1;
            } else if kind == dex::EffectType::Move || e == EffectRef::None {
                len = 0;
            } else {
                tags[0] = if id == dex::CONDITION_TOX {
                    LogTag::Value("from", LogArg::Text("psn"))
                } else {
                    LogTag::From(e)
                };
                len = if let Some(s) =
                    source.filter(|s| *s != t || kind == dex::EffectType::Ability)
                {
                    tags[1] = LogTag::Of(s);
                    2
                } else {
                    1
                };
            }
            self.add(LogEntry::split(
                "-damage",
                &[LogArg::Mon(t), LogArg::Health(t)],
                &tags[..len],
                t.side(),
                false,
            ));
            if actual != 0. && kind == dex::EffectType::Move {
                let drain = match e {
                    EffectRef::ActiveMove(h) => self.scratch.moves[h as usize].unwrap().drain,
                    EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
                        dex::move_data(id).drain
                    }
                    _ => None,
                };
                if let (Some([num, den]), Some(s)) = (drain, source) {
                    self.heal(
                        (actual * num as f64 / den as f64).round(),
                        Some(s),
                        Some(t),
                        HealEffect::Drain,
                    );
                }
            }
        }
        if instafaint {
            for i in 0..targets.len as usize {
                if result.values[i].truthy() {
                    if let HitTarget::Pokemon(t) = targets.entries[i] {
                        if self.state.pokemon[t.0 as usize].hp == 0 {
                            self.faint_messages(true, false, true);
                        }
                    }
                }
            }
        }
        result
    }
    /// Bypasses Damage hooks; uses direct damage attribution/log rules
    /// Ports `sim/battle.ts:2211-2263`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn direct_damage(
        &mut self,
        amount: f64,
        target: Option<MonId>,
        attribution: Attribution,
    ) -> Relay {
        let a = self.mutation_attribution(attribution, false, true);
        let target = target.or_else(|| self.mutation_frame().and_then(|f| Self::arg_mon(f.target)));
        let Some(t) = target else {
            return Relay::Number(0.);
        };
        if self.state.pokemon[t.0 as usize].hp == 0 || amount == 0. || amount.is_nan() {
            return Relay::Number(0.);
        }
        let d = self.raw_damage(t, amount.floor().max(1.), a);
        let tags: &[LogTag<'_>] = match a.effect {
            EffectRef::Synthetic(SyntheticEffect::StruggleRecoil) => {
                &[LogTag::Value("from", LogArg::Text("recoil"))]
            }
            _ if self.event_effect_id(a.effect) == dex::CONDITION_CONFUSION => {
                &[LogTag::Value("from", LogArg::Text("confusion"))]
            }
            _ => &[],
        };
        self.add(LogEntry::split(
            "-damage",
            &[LogArg::Mon(t), LogArg::Health(t)],
            tags,
            t.side(),
            false,
        ));
        if self.state.pokemon[t.0 as usize].flags & mon_flags::FAINTED != 0 {
            self.faint(t, Attribution::NONE);
        }
        Relay::Number(d)
    }
    /// True/false/zero/number returns and drain attribution
    /// Ports `sim/battle.ts:2265-2307`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn heal(
        &mut self,
        amount: f64,
        target: Option<MonId>,
        source: Option<MonId>,
        effect: HealEffect,
    ) -> Relay {
        let effect = match effect {
            HealEffect::Context => EffectRef::None,
            HealEffect::Effect(e) => e,
            HealEffect::Drain => EffectRef::Dex(dex::CONDITION_DRAIN),
        };
        let a = self.mutation_attribution(
            Attribution {
                source: mon_arg(source),
                effect,
            },
            false,
            true,
        );
        let target = target.or_else(|| self.mutation_frame().and_then(|f| Self::arg_mon(f.target)));
        let n = crate::math::trunc_f64(if amount != 0. && amount <= 1. {
            1.
        } else {
            amount
        }) as f64;
        let r = self.mutation_event(EventId::TryHeal, mon_arg(target), a, Relay::Number(n));
        if !r.truthy() {
            return r;
        }
        let Some(t) = target else {
            return Relay::Bool(false);
        };
        let p = &self.state.pokemon[t.0 as usize];
        if p.hp == 0 || p.flags & mon_flags::ACTIVE == 0 || p.hp >= p.max_hp {
            return Relay::Bool(false);
        }
        let result = self.raw_heal(t, number(r));
        let id = self.event_effect_id(a.effect);
        let source = Self::arg_mon(a.source);
        if id != dex::CONDITION_WISH && a.effect != EffectRef::None {
            let mut tags = [LogTag::Bare("silent"); 2];
            let len;
            if id == dex::CONDITION_LEECHSEED || id == dex::MOVE_REST {
                len = 1;
            } else if id == dex::CONDITION_DRAIN {
                tags[0] = LogTag::Value("from", LogArg::Text("drain"));
                tags[1] = LogTag::Value("of", source.map_or(LogArg::Text("null"), LogArg::Mon));
                len = 2;
            } else if self.event_effect_type(a.effect) == dex::EffectType::Move {
                len = 0;
            } else {
                tags[0] = LogTag::From(a.effect);
                len = if let Some(s) = source.filter(|s| *s != t) {
                    tags[1] = LogTag::Of(s);
                    2
                } else {
                    1
                };
            }
            self.add(LogEntry::split(
                "-heal",
                &[LogArg::Mon(t), LogArg::Health(t)],
                &tags[..len],
                t.side(),
                false,
            ));
        }
        self.mutation_event(EventId::Heal, mon_arg(Some(t)), a, result);
        result
    }
    /// Pokemon.damage applies HP bounds and queues faint
    /// Ports `sim/pokemon.ts:1595-1605`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn raw_damage(&mut self, pokemon: MonId, amount: f64, attribution: Attribution) -> f64 {
        if self.state.pokemon[pokemon.0 as usize].hp == 0 || amount.is_nan() || amount <= 0. {
            return 0.;
        }
        let d = crate::math::trunc_f64(if amount < 1. { 1. } else { amount });
        let hp = self.state.pokemon[pokemon.0 as usize].hp as u32;
        if d >= hp {
            self.faint_pokemon(pokemon, attribution);
            hp as f64
        } else {
            self.state.pokemon[pokemon.0 as usize].hp -= d as u16;
            d as f64
        }
    }
    /// Pokemon.heal only; no battle events/log line
    /// Ports `sim/pokemon.ts:1640-1653`. PRNG: none.
    pub fn raw_heal(&mut self, pokemon: MonId, amount: f64) -> Relay {
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        if p.hp == 0 {
            return Relay::Bool(false);
        }
        let d = crate::math::trunc_f64(amount);
        if d == 0 || p.hp >= p.max_hp {
            return Relay::Bool(false);
        }
        let actual = d.min(u32::from(p.max_hp - p.hp)) as u16;
        p.hp += actual;
        Relay::Number(actual as f64)
    }
    /// Queues faint once; lifecycle later drains the queue
    /// Ports `sim/pokemon.ts:1581-1593`. PRNG: none.
    pub fn faint_pokemon(&mut self, pokemon: MonId, attribution: Attribution) -> f64 {
        let effect = self.freeze_effect(attribution.effect);
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        if p.flags & (mon_flags::FAINTED | mon_flags::FAINT_QUEUED) != 0 {
            return 0.;
        }
        let hp = p.hp;
        p.hp = 0;
        p.switch_flag = EffectId::NONE;
        p.flags &= !mon_flags::SWITCH_REQUESTED;
        p.flags |= mon_flags::FAINT_QUEUED;
        let i = self.state.faint_queue_len as usize;
        self.state.faint_queue[i] = crate::state::FaintEntry {
            target: pokemon,
            source: Self::arg_mon(attribution.source).unwrap_or(MonId::NONE),
            effect,
        };
        self.state.faint_queue_len += 1;
        hp as f64
    }
    /// Battle.faint delegates to Pokemon faint queueing
    /// Ports `sim/battle.ts:1623-1625`. PRNG: none.
    pub fn faint(&mut self, pokemon: MonId, attribution: Attribution) -> () {
        self.faint_pokemon(pokemon, attribution);
    }
    /// Sparse insertion order drives TryBoost/Boost/AfterEachBoost/logs
    /// Ports `sim/battle.ts:2024-2093`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn boost(
        &mut self,
        boosts: OrderedBoosts,
        target: Option<MonId>,
        attribution: Attribution,
        is_secondary: bool,
        is_self: bool,
    ) -> Relay {
        let a = self.mutation_attribution(attribution, false, true);
        let target = target.or_else(|| self.mutation_frame().and_then(|f| Self::arg_mon(f.target)));
        let Some(t) = target else {
            return Relay::Number(0.);
        };
        let p = &self.state.pokemon[t.0 as usize];
        if p.hp == 0 {
            return Relay::Number(0.);
        }
        if p.flags & mon_flags::ACTIVE == 0
            || self.state.sides[(1 - t.side().0) as usize].pokemon_left == 0
        {
            return Relay::Bool(false);
        }
        let initial = self.stash_boosts(boosts);
        let r = self.mutation_event(
            EventId::ChangeBoost,
            mon_arg(Some(t)),
            a,
            Relay::Boosts(initial),
        );
        let b = if let Relay::Boosts(h) = r {
            *self.scratch_boosts(h)
        } else {
            OrderedBoosts::default()
        };
        if r != Relay::Boosts(initial) {
            self.release_relay(r);
        }
        self.release_relay(Relay::Boosts(initial));
        let capped = self.get_capped_boost(t, b);
        let input = self.stash_boosts(capped);
        let r = self.mutation_event(EventId::TryBoost, mon_arg(Some(t)), a, Relay::Boosts(input));
        let b = if let Relay::Boosts(h) = r {
            *self.scratch_boosts(h)
        } else {
            OrderedBoosts::default()
        };
        if r != Relay::Boosts(input) {
            self.release_relay(r);
        }
        self.release_relay(Relay::Boosts(input));
        let mut success = Relay::Null;
        let mut boosted = is_secondary;
        for &stat in &b.order[..b.len as usize] {
            let mut one = OrderedBoosts::default();
            one.values[stat as usize] = b.values[stat as usize];
            one.order[0] = stat;
            one.len = 1;
            one.present = 1 << stat;
            let delta = self.boost_by(t, one);
            let negative = b.values[stat as usize] < 0
                || self.state.pokemon[t.0 as usize].boosts[stat as usize] == -6;
            let by = if negative { -delta } else { delta };
            let msg = if negative { "-unboost" } else { "-boost" };
            let name = ["atk", "def", "spa", "spd", "spe", "accuracy", "evasion"][stat as usize];
            let kind = self.event_effect_type(a.effect);
            let id = self.event_effect_id(a.effect);
            if by != 0 {
                success = Relay::Bool(true);
                if id == dex::MOVE_BELLYDRUM
                    || id != EffectId::NONE && dex::effect(id).key == "angerpoint"
                {
                    self.add(LogEntry::new(
                        "-setboost",
                        &[
                            LogArg::Mon(t),
                            LogArg::Text("atk"),
                            LogArg::Number(self.state.pokemon[t.0 as usize].boosts[0] as i32),
                        ],
                        &[LogTag::From(a.effect)],
                    ));
                } else if a.effect != EffectRef::None {
                    if kind == dex::EffectType::Ability && !boosted {
                        self.add(LogEntry::new(
                            "-ability",
                            &[
                                LogArg::Mon(t),
                                LogArg::Effect(a.effect),
                                LogArg::Text("boost"),
                            ],
                            &[],
                        ));
                        boosted = true;
                    }
                    let tags: &[LogTag<'_>] = if kind == dex::EffectType::Item {
                        &[LogTag::From(a.effect)]
                    } else {
                        &[]
                    };
                    self.add(LogEntry::new(
                        msg,
                        &[
                            LogArg::Mon(t),
                            LogArg::Text(name),
                            LogArg::Number(by as i32),
                        ],
                        tags,
                    ));
                }
                let h = self.stash_boosts(one);
                let r = self.mutation_event(
                    EventId::AfterEachBoost,
                    mon_arg(Some(t)),
                    a,
                    Relay::Boosts(h),
                );
                if r != Relay::Boosts(h) {
                    self.release_relay(r);
                }
                self.release_relay(Relay::Boosts(h));
            } else if if kind == dex::EffectType::Ability {
                is_secondary || is_self
            } else {
                !is_secondary && !is_self
            } {
                self.add(LogEntry::new(
                    msg,
                    &[
                        LogArg::Mon(t),
                        LogArg::Text(name),
                        LogArg::Number(by as i32),
                    ],
                    &[],
                ));
            }
        }
        let h = self.stash_boosts(b);
        let r = self.mutation_event(EventId::AfterBoost, mon_arg(Some(t)), a, Relay::Boosts(h));
        let b = *self.scratch_boosts(h);
        if r != Relay::Boosts(h) {
            self.release_relay(r);
        }
        self.release_relay(Relay::Boosts(h));
        if success.truthy() {
            let p = &mut self.state.pokemon[t.0 as usize];
            if b.values.iter().any(|n| *n > 0) {
                p.flags |= mon_flags::STATS_RAISED;
            }
            if b.values.iter().any(|n| *n < 0) {
                p.flags |= mon_flags::STATS_LOWERED;
            }
        }
        success
    }
    /// Clamp requested stages; return the last actual delta
    /// Ports `sim/pokemon.ts:1221-1230`. PRNG: none.
    pub fn boost_by(&mut self, pokemon: MonId, boosts: OrderedBoosts) -> i8 {
        let b = self.get_capped_boost(pokemon, boosts);
        let mut delta = 0;
        for &i in &b.order[..b.len as usize] {
            delta = b.values[i as usize];
            self.state.pokemon[pokemon.0 as usize].boosts[i as usize] += delta;
        }
        delta
    }
    /// Set only present boost properties
    /// Ports `sim/pokemon.ts:1239-1244`. PRNG: none.
    pub fn set_boost(&mut self, pokemon: MonId, boosts: OrderedBoosts) -> () {
        for &i in &boosts.order[..boosts.len as usize] {
            self.state.pokemon[pokemon.0 as usize].boosts[i as usize] = boosts.values[i as usize];
        }
    }
    /// Clear all seven boost stages
    /// Ports `sim/pokemon.ts:1232-1237`. PRNG: none.
    pub fn clear_boosts(&mut self, pokemon: MonId) -> () {
        self.state.pokemon[pokemon.0 as usize].boosts = [0; 7];
    }
    /// Mark used PP slot; return actual deduction or zero
    /// Ports `sim/pokemon.ts:888-902`. PRNG: none.
    pub fn deduct_pp(&mut self, pokemon: MonId, move_id: EffectId, amount: Option<f64>) -> f64 {
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        let slots = if p.flags & mon_flags::TRANSFORMED != 0 {
            &mut p.virtual_move_slots[..p.virtual_move_count as usize]
        } else {
            &mut p.base_move_slots[..p.move_count as usize]
        };
        let Some(slot) = slots.iter_mut().find(|s| s.id == move_id) else {
            return 0.;
        };
        slot.flags |= super::pokemon::SLOT_USED;
        if slot.pp == 0 {
            return 0.;
        }
        let amount = amount.filter(|n| *n != 0. && !n.is_nan()).unwrap_or(1.);
        let actual = amount.min(slot.pp as f64);
        slot.pp = (slot.pp as f64 - actual) as u8;
        actual
    }
    /// Encode current-turn/sourceSlot and numeric-damage fallback exactly
    /// Ports `sim/pokemon.ts:917-928`. PRNG: none.
    pub fn got_attacked(
        &mut self,
        target: MonId,
        move_id: EffectId,
        damage: Relay,
        source: MonId,
    ) -> () {
        let numeric = if let Relay::Number(n) = damage {
            Some(n as u32)
        } else {
            None
        };
        self.state.record_attack(target, source, move_id, numeric);
    }
}
impl<L: LogSink> Battle<L> {
    /// Ports sim/pokemon.ts:1656-1667. PRNG: none; undefined on NaN, otherwise HP delta.
    pub fn set_hp(&mut self, pokemon: MonId, amount: f64) -> Relay {
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        if p.hp == 0 {
            return Relay::Number(0.);
        }
        let new = crate::math::trunc_f64(amount).max(1).min(p.max_hp as u32) as u16;
        let delta = new as f64 - p.hp as f64;
        p.hp = new;
        Relay::Number(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_hp_ports_match_pinned_oracle_including_uint32_wrapping() {
        let packed = "Pikachu|||static|thunderbolt|Serious||M|||100|,,,,,Electric";
        for line in include_str!("../../../../../tools/probes/mutators/raw.tsv").lines() {
            let v: Vec<_> = line.split('\t').collect();
            let n = match v[1] {
                "NaN" => f64::NAN,
                "Infinity" => f64::INFINITY,
                s => s.parse().unwrap(),
            };
            let mut b = Battle::<crate::log::NoLog>::new([1, 2, 3, 4], packed, packed).unwrap();
            b.state.pokemon[0].hp = 100;
            b.state.pokemon[0].max_hp = 120;
            let r = match v[0] {
                "damage" => Relay::Number(b.raw_damage(MonId(0), n, Attribution::NONE)),
                "heal" => b.raw_heal(MonId(0), n),
                "sethp" => b.set_hp(MonId(0), n),
                _ => unreachable!(),
            };
            let expected = match v[2] {
                "false" => Relay::Bool(false),
                "undefined" => Relay::Undefined,
                s => Relay::Number(s.parse().unwrap()),
            };
            assert_eq!(r, expected, "{line}");
            assert_eq!(
                b.state.pokemon[0].hp,
                v[3].parse::<u16>().unwrap(),
                "{line}"
            );
            assert_eq!(
                b.state.faint_queue_len,
                v[4].parse::<u8>().unwrap(),
                "{line}"
            );
        }
    }
}
