//! Pokemon stat/weight and per-slot hit metadata queries from pinned Showdown.
use crate::{
    Battle,
    actions::{MoveHandle, Stat, StatOptions},
    dex::{self, EventId},
    event::{EffectRef, EventArg, Relay, RunEventOptions},
    ids::*,
    log::LogSink,
    math,
    state::scratch::{HitData, OrderedBoosts},
};
impl<L: LogSink> Battle<L> {
    /// Wonder Room then sparse ModifyBoost and explicit modifier.
    /// Ports `sim/pokemon.ts:560-594`. PRNG: dispatched events/callbacks only.
    pub fn calculate_stat(
        &mut self,
        pokemon: MonId,
        stat: Stat,
        boost: i8,
        modifier: Option<f64>,
        stat_user: Option<MonId>,
    ) -> f64 {
        let mut base_index = stat as usize;
        // calculateStat swaps the base defense, but leaves the boost key intact.
        if self.stats_has_pseudo_weather(dex::key_ids!("wonderroom")) {
            base_index = opposite_defense(stat) as usize;
        }
        let base = self.state.pokemon[pokemon.0 as usize].stored_stats[base_index];
        let index = stat as usize;
        let mut boosts = OrderedBoosts::default();
        boosts.values[index] = boost;
        boosts.order[0] = index as u8;
        boosts.len = 1;
        boosts.present = 1 << index;
        let boosts = self.stats_modify_boosts(stat_user.unwrap_or(pokemon), boosts);
        let stage = (boosts.present & (1 << index) != 0).then_some(boosts.values[index]);
        let stat = boosted_stat(f64::from(base), stage);
        // JS modifier || 1: explicit zero and NaN select 1, negatives do not.
        let modifier = modifier.filter(|n| *n != 0.0 && !n.is_nan()).unwrap_or(1.0);
        self.modify(stat, modifier, 1.0)
    }
    /// Mutable event dispatch despite being a stat query.
    /// Ports `sim/pokemon.ts:596-639`. PRNG: dispatched events/callbacks only.
    pub fn get_stat(&mut self, pokemon: MonId, mut stat: Stat, options: StatOptions) -> f64 {
        // Download/unmodified reads the original base, then swaps ONLY the boost
        // key in Wonder Room. Ordinary getStat leaves the swap to ModifyDef/SpD.
        let mut value =
            f64::from(self.state.pokemon[pokemon.0 as usize].stored_stats[stat as usize]);
        if options.unmodified && self.stats_has_pseudo_weather(dex::key_ids!("wonderroom")) {
            stat = opposite_defense(stat);
        }
        if !options.unboosted {
            let mut boosts = OrderedBoosts {
                values: self.state.pokemon[pokemon.0 as usize].boosts,
                order: [0, 1, 2, 3, 4, 5, 6],
                len: 7,
                present: 0x7f,
            };
            if !options.unmodified {
                boosts = self.stats_modify_boosts(pokemon, boosts);
            }
            let index = stat as usize;
            let stage = (boosts.present & (1 << index) != 0).then_some(boosts.values[index]);
            value = boosted_stat(value, stage);
        }
        if !options.unmodified {
            let event = match stat {
                Stat::Atk => EventId::ModifyAtk,
                Stat::Def => EventId::ModifyDef,
                Stat::SpA => EventId::ModifySpA,
                Stat::SpD => EventId::ModifySpD,
                Stat::Spe => EventId::ModifySpe,
            };
            value = if stat == Stat::Spe && self.query_event_is_empty(pokemon, event) {
                numeric_stat_relay(self.empty_query_event(event, Relay::Number(value)))
            } else {
                numeric_stat_relay(self.run_event(
                    event,
                    EventArg::Holder(Holder::mon(pokemon)),
                    EventArg::Null,
                    EffectRef::None,
                    Relay::Number(value),
                    RunEventOptions::default(),
                ))
            };
        }
        // This format does not override battle.trunc.
        if stat == Stat::Spe && value > 10000.0 {
            value = 10000.0;
        }
        value
    }
    /// Trick Room reverses modified speed before the uint32/13-bit wrap.
    /// Ports `sim/pokemon.ts:641-649`. PRNG: dispatched events/callbacks only.
    pub fn pokemon_action_speed(&mut self, pokemon: MonId) -> u16 {
        let mut speed = self.get_stat(pokemon, Stat::Spe, StatOptions::default());
        // gen9randomdoublesbattle does not include twisteddimensionmod, so the
        // source's ruleTable ternary selects the ordinary Trick Room presence.
        if self.stats_has_pseudo_weather(dex::key_ids!("trickroom")) {
            speed = 10000.0 - speed;
        }
        (math::trunc_f64(speed) % (1 << 13)) as u16
    }
    /// Refresh cached speed with getActionSpeed, including Trick Room and wrap.
    /// Ports `sim/pokemon.ts:556-558`. PRNG: dispatched events/callbacks only.
    pub fn update_pokemon_speed(&mut self, pokemon: MonId) {
        let speed = self.pokemon_action_speed(pokemon);
        self.state.pokemon[pokemon.0 as usize].speed = speed;
    }
    /// Hit metadata belongs to this move-use frame and CURRENT slot, not MonId.
    /// Ports `sim/pokemon.ts:520-523,706-714`. PRNG: none.
    pub fn move_hit_data(&mut self, target: MonId, move_handle: MoveHandle) -> &mut HitData {
        let slot = usize::from(target.side().0) * 6
            + usize::from(self.state.pokemon[target.0 as usize].position);
        let data = &mut self.active_move_mut(move_handle).hit_data[slot];
        if !data.present {
            *data = HitData {
                present: true,
                ..HitData::default()
            };
        }
        data
    }
    /// Preserve the repeated getStat call when each new maximum is found.
    /// Ports `sim/pokemon.ts:656-668`. PRNG: dispatched events/callbacks only.
    pub fn get_best_stat(&mut self, pokemon: MonId, options: StatOptions) -> Stat {
        let mut stat = Stat::Atk;
        let mut best = 0.0;
        for candidate in [Stat::Atk, Stat::Def, Stat::SpA, Stat::SpD, Stat::Spe] {
            if self.get_stat(pokemon, candidate, options) > best {
                stat = candidate;
                best = self.get_stat(pokemon, candidate, options);
            }
        }
        stat
    }
    /// ModifyWeight then Math.max(1, weight), including NaN preservation.
    /// Ports `sim/pokemon.ts:685-688`. PRNG: dispatched events/callbacks only.
    pub fn get_weight(&mut self, pokemon: MonId) -> f64 {
        let weight = f64::from(self.state.pokemon[pokemon.0 as usize].weighthg);
        let weight = numeric_stat_relay(self.run_event(
            EventId::ModifyWeight,
            EventArg::Holder(Holder::mon(pokemon)),
            EventArg::Null,
            EffectRef::None,
            Relay::Number(weight),
            RunEventOptions::default(),
        ));
        if weight.is_nan() {
            weight
        } else {
            weight.max(1.0)
        }
    }

    /// Any pseudo-weather cell whose effect carries one of `keys` (compile-time ids).
    fn stats_has_pseudo_weather(&self, keys: dex::KeyIds) -> bool {
        self.state
            .field
            .pseudo_weather
            .as_slice()
            .iter()
            .any(|c| keys.contains(self.state.effects.cells[c.0 as usize].id))
    }
    /// The source receives a fresh object: sparse in calculateStat, full copy in
    /// getStat. A callback may replace it; reclaim both handles after copying.
    /// Ports `sim/pokemon.ts:580,618`. PRNG: ModifyBoost dispatch only.
    fn stats_modify_boosts(&mut self, target: MonId, boosts: OrderedBoosts) -> OrderedBoosts {
        let original = self.stash_boosts(boosts);
        let result = if self.query_event_is_empty(target, EventId::ModifyBoost) {
            self.empty_query_event(EventId::ModifyBoost, Relay::Boosts(original))
        } else {
            self.run_event(
                EventId::ModifyBoost,
                EventArg::Holder(Holder::mon(target)),
                EventArg::Null,
                EffectRef::None,
                Relay::Boosts(original),
                RunEventOptions::default(),
            )
        };
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
}
fn opposite_defense(stat: Stat) -> Stat {
    match stat {
        Stat::Def => Stat::SpD,
        Stat::SpD => Stat::Def,
        stat => stat,
    }
}
fn boosted_stat(base: f64, stage: Option<i8>) -> f64 {
    // A deleted boost key is JS undefined; preserve NaN rather than inventing
    // an unboosted zero stage. calculateStat's final modify then truncates NaN.
    let Some(stage) = stage else {
        return f64::NAN;
    };
    let stage = stage.clamp(-6, 6);
    let multiplier = 1.0 + f64::from(stage.abs()) / 2.0;
    if stage >= 0 {
        (base * multiplier).floor()
    } else {
        (base / multiplier).floor()
    }
}
fn numeric_stat_relay(value: Relay) -> f64 {
    let Relay::Number(value) = value else {
        panic!("stat/weight event returned a nonnumeric relay");
    };
    value
}
#[cfg(test)]
#[path = "stats/tests.rs"]
mod tests;
