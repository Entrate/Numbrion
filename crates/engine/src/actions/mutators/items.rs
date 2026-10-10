//! Item/ability queries and mutations ported from pinned Showdown.
#![allow(unused_variables, unused_imports)]
use super::common::{ATE_BERRY, USED_ITEM_THIS_TURN, mon_arg, number};
use crate::{
    Battle,
    actions::*,
    dex::{self, ImmunityId},
    event::{EffectRef, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{
        CellId, mon_flags, present,
        scratch::{HitData, OrderedBoosts},
    },
};
impl<L: LogSink> Battle<L> {
    /// Item relay or false; TakeItem veto, old item-state identity
    /// Ports `sim/pokemon.ts:1851-1866`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn take_item(&mut self, pokemon: MonId, source: Option<MonId>) -> Relay {
        let source = source.unwrap_or(pokemon);
        let item = self.state.pokemon[pokemon.0 as usize].item;
        if item == EffectId::NONE {
            return Relay::Undefined;
        }
        let r = self.mutation_event(
            EventId::TakeItem,
            mon_arg(Some(pokemon)),
            Attribution {
                source: mon_arg(Some(source)),
                effect: EffectRef::None,
            },
            Relay::Effect(item),
        );
        if !r.truthy() {
            return Relay::Bool(false);
        }
        self.state.pokemon[pokemon.0 as usize].item = EffectId::NONE;
        let cell = self.state.pokemon[pokemon.0 as usize].item_state;
        self.state.effects.cells[cell.0 as usize].clear();
        self.mutation_single(
            EventId::End,
            EffectRef::Dex(item),
            cell,
            mon_arg(Some(pokemon)),
            Attribution::DEFAULT,
        );
        self.mutation_event(
            EventId::AfterTakeItem,
            mon_arg(Some(pokemon)),
            Attribution::NONE,
            Relay::Effect(item),
        );
        Relay::Effect(item)
    }
    /// Install item state and Start when appropriate
    /// Ports `sim/pokemon.ts:1868-1893`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn set_item(&mut self, pokemon: MonId, item: EffectId, attribution: Attribution) -> bool {
        let p = &self.state.pokemon[pokemon.0 as usize];
        if p.hp == 0 || p.flags & mon_flags::ACTIVE == 0 {
            return false;
        }
        let old = p.item;
        let old_cell = p.item_state;
        let cell = self.mutation_cell(Holder::mon(pokemon), Holder::mon(pokemon), item);
        self.state.pokemon[pokemon.0 as usize].item = item;
        self.state.pokemon[pokemon.0 as usize].item_state = cell;
        if old != EffectId::NONE {
            self.mutation_single(
                EventId::End,
                EffectRef::Dex(old),
                old_cell,
                mon_arg(Some(pokemon)),
                Attribution::DEFAULT,
            );
        }
        self.release_cell(old_cell);
        if item != EffectId::NONE {
            self.mutation_single(
                EventId::Start,
                EffectRef::Dex(item),
                cell,
                mon_arg(Some(pokemon)),
                attribution,
            );
        }
        true
    }
    /// UseItem then AfterUseItem, state flags and enditem line
    /// Ports `sim/pokemon.ts:1811-1849`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn use_item(&mut self, pokemon: MonId, attribution: Attribution) -> bool {
        let p = &self.state.pokemon[pokemon.0 as usize];
        let item = p.item;
        if item == EffectId::NONE {
            return false;
        }
        let gem = dex::ITEMS[(item.0 - dex::ITEM_START) as usize].gem;
        if (p.hp == 0 && !gem) || p.flags & mon_flags::ACTIVE == 0 {
            return false;
        }
        let a = self.mutation_attribution(attribution, true, true);
        if self.event_effect_type(a.effect) == dex::EffectType::Item
            && self.event_effect_id(a.effect) != item
            && Self::arg_mon(a.source) == Some(pokemon)
        {
            return false;
        }
        if !self
            .mutation_event(
                EventId::UseItem,
                mon_arg(Some(pokemon)),
                Attribution::NONE,
                Relay::Effect(item),
            )
            .truthy()
        {
            return false;
        }
        if dex::effect(item).key == "redcard" {
            self.add(LogEntry::new(
                "-enditem",
                &[LogArg::Mon(pokemon), LogArg::Effect(EffectRef::Dex(item))],
                &[LogTag::Value(
                    "of",
                    Self::arg_mon(a.source).map_or(LogArg::Text("undefined"), LogArg::Mon),
                )],
            ));
        } else if gem {
            self.add(LogEntry::new(
                "-enditem",
                &[LogArg::Mon(pokemon), LogArg::Effect(EffectRef::Dex(item))],
                &[
                    LogTag::Value("from", LogArg::Text("gem")),
                    LogTag::Value(
                        "move",
                        LogArg::Effect(EffectRef::ActiveMove(self.scratch.active_move.0)),
                    ),
                ],
            ));
        } else {
            self.add(LogEntry::new(
                "-enditem",
                &[LogArg::Mon(pokemon), LogArg::Effect(EffectRef::Dex(item))],
                &[],
            ));
        }
        if let Some(dex::DataValue::Object(properties)) =
            dex::effect(item).data.get(dex::FIELD_BOOSTS)
        {
            let mut boosts = OrderedBoosts::default();
            for prop in properties {
                let stat = match prop.key {
                    dex::FIELD_ATK => 0,
                    dex::FIELD_DEF => 1,
                    dex::FIELD_SPA => 2,
                    dex::FIELD_SPD => 3,
                    dex::FIELD_SPE => 4,
                    dex::FIELD_ACCURACY => 5,
                    _ => continue,
                };
                if let dex::DataValue::Number(n) = prop.value {
                    boosts.values[stat] = n as i8;
                    boosts.order[boosts.len as usize] = stat as u8;
                    boosts.len += 1;
                    boosts.present |= 1 << stat;
                }
            }
            self.boost(
                boosts,
                Some(pokemon),
                Attribution {
                    effect: EffectRef::Dex(item),
                    ..a
                },
                false,
                false,
            );
        }
        let cell = self.state.pokemon[pokemon.0 as usize].item_state;
        self.mutation_single(
            EventId::Use,
            EffectRef::Dex(item),
            cell,
            mon_arg(Some(pokemon)),
            a,
        );
        self.consume_item(pokemon, item);
        self.mutation_event(
            EventId::AfterUseItem,
            mon_arg(Some(pokemon)),
            Attribution::NONE,
            Relay::Effect(item),
        );
        true
    }
    /// UseItem/TryEatItem/EatItem/Eat, End and consumption bits
    /// Ports `sim/pokemon.ts:1768-1809`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn eat_item(&mut self, pokemon: MonId, attribution: Attribution, force: bool) -> bool {
        let p = &self.state.pokemon[pokemon.0 as usize];
        let item = p.item;
        if item == EffectId::NONE {
            return false;
        }
        let after_faint = matches!(dex::effect(item).key, "jabocaberry" | "rowapberry");
        if (p.hp == 0 && !after_faint) || p.flags & mon_flags::ACTIVE == 0 {
            return false;
        }
        let a = self.mutation_attribution(attribution, true, true);
        if self.event_effect_type(a.effect) == dex::EffectType::Item
            && self.event_effect_id(a.effect) != item
            && Self::arg_mon(a.source) == Some(pokemon)
        {
            return false;
        }
        if !self
            .mutation_event(
                EventId::UseItem,
                mon_arg(Some(pokemon)),
                Attribution::NONE,
                Relay::Effect(item),
            )
            .truthy()
            || (!force
                && !self
                    .mutation_event(
                        EventId::TryEatItem,
                        mon_arg(Some(pokemon)),
                        Attribution::NONE,
                        Relay::Effect(item),
                    )
                    .truthy())
        {
            return false;
        }
        self.add(LogEntry::new(
            "-enditem",
            &[LogArg::Mon(pokemon), LogArg::Effect(EffectRef::Dex(item))],
            &[LogTag::Bare("eat")],
        ));
        let cell = self.state.pokemon[pokemon.0 as usize].item_state;
        self.mutation_single(
            EventId::Eat,
            EffectRef::Dex(item),
            cell,
            mon_arg(Some(pokemon)),
            a,
        );
        self.mutation_event(
            EventId::EatItem,
            mon_arg(Some(pokemon)),
            a,
            Relay::Effect(item),
        );
        self.consume_item(pokemon, item);
        self.state.pokemon[pokemon.0 as usize].flags |= ATE_BERRY;
        self.mutation_event(
            EventId::AfterUseItem,
            mon_arg(Some(pokemon)),
            Attribution::NONE,
            Relay::Effect(item),
        );
        true
    }
    /// Shared lastItem/itemUsed/ateBerry bookkeeping; does not synthesize events
    /// Ports `sim/pokemon.ts:1800-1807,1841-1847`. PRNG: none.
    pub fn consume_item(&mut self, pokemon: MonId, item: EffectId) -> () {
        let p = &mut self.state.pokemon[pokemon.0 as usize];
        // Eat/Use callbacks can replace the held item; TS captures the live item here.
        p.last_item = p.item;
        p.item = EffectId::NONE;
        p.flags |= USED_ITEM_THIS_TURN;
        self.state.effects.cells[p.item_state.0 as usize].clear();
    }
    /// Delegate set_item empty
    /// Ports `sim/pokemon.ts:1904-1906`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn clear_item(&mut self, pokemon: MonId) -> bool {
        self.set_item(pokemon, EffectId::NONE, Attribution::DEFAULT)
    }
    /// Match the item first, then honor ignoringItem
    /// Ports `sim/pokemon.ts:1895-1902`. PRNG: none.
    pub fn has_item(&self, pokemon: MonId, items: &[EffectId]) -> bool {
        items.contains(&self.state.pokemon[pokemon.0 as usize].item) && !self.ignoring_item(pokemon)
    }
    /// Embargo/Klutz/Magic Room and fainted/active distinctions
    /// Ports `sim/pokemon.ts:879-886`. PRNG: none.
    pub fn ignoring_item(&self, pokemon: MonId) -> bool {
        let mon = &self.state.pokemon[pokemon.0 as usize];
        // pokemon.ts:880: Primal Orbs precede even the inactive check. Read the
        // generated property because ItemData deliberately omits this rare field.
        if mon.item != EffectId::NONE
            && dex::effect(mon.item).data.get(dex::FIELD_ISPRIMALORB)
                == Some(dex::DataValue::Bool(true))
        {
            return false;
        }
        if mon.flags & mon_flags::ACTIVE == 0 {
            return true;
        }
        if self.query_has_volatile(pokemon, "embargo")
            || self.state.field.pseudo_weather.cells[..self.state.field.pseudo_weather.len as usize]
                .iter()
                .any(|c| query_effect_is(self.state.effects.cells[c.0 as usize].id, "magicroom"))
        {
            return true;
        }
        let ignore_klutz = mon.item != EffectId::NONE
            && dex::ITEMS[(mon.item.0 - dex::ITEM_START) as usize].ignore_klutz;
        !ignore_klutz && self.query_has_ability(pokemon, "klutz")
    }
    /// Old-ability relay/false/null; End/Start and suppression restrictions
    /// Ports `sim/pokemon.ts:1908-1950`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn set_ability(
        &mut self,
        pokemon: MonId,
        ability: EffectId,
        attribution: Attribution,
        is_from_forme_change: bool,
        is_transform: bool,
    ) -> Relay {
        if self.state.pokemon[pokemon.0 as usize].hp == 0 {
            return Relay::Bool(false);
        }
        let mut a = attribution;
        if a.effect == EffectRef::None {
            a.effect = if self.scratch.current_effect == EffectRef::None {
                EffectRef::Dex(EffectId::NONE)
            } else {
                self.scratch.current_effect
            };
        }
        let old = self.state.pokemon[pokemon.0 as usize].ability;
        let cant = |id: EffectId| {
            id != EffectId::NONE
                && dex::ABILITIES[(id.0 - dex::ABILITY_START) as usize].flags
                    & dex::FLAG_CANTSUPPRESS
                    != 0
        };
        if !is_from_forme_change && (cant(ability) || cant(old)) {
            return Relay::Bool(false);
        }
        if !is_from_forme_change && !is_transform {
            let r = self.mutation_event(
                EventId::SetAbility,
                mon_arg(Some(pokemon)),
                a,
                Relay::Effect(ability),
            );
            if !r.truthy() {
                return r;
            }
        }
        let old_cell = self.state.pokemon[pokemon.0 as usize].ability_state;
        self.mutation_single(
            EventId::End,
            EffectRef::Dex(old),
            old_cell,
            mon_arg(Some(pokemon)),
            Attribution {
                effect: EffectRef::None,
                ..a
            },
        );
        let cell = self.mutation_cell(Holder::mon(pokemon), Holder::mon(pokemon), ability);
        self.state.pokemon[pokemon.0 as usize].ability = ability;
        self.state.pokemon[pokemon.0 as usize].ability_state = cell;
        self.release_cell(old_cell);
        if a.effect != EffectRef::None && !is_from_forme_change && !is_transform {
            let id = self.event_effect_id(a.effect);
            if id != EffectId::NONE && matches!(dex::effect(id).key, "mummy" | "lingeringaroma") {
                self.add(LogEntry::new(
                    "-activate",
                    &[
                        Self::arg_mon(a.source).map_or(LogArg::Text("undefined"), LogArg::Mon),
                        LogArg::EffectFullName(a.effect),
                        LogArg::Mon(pokemon),
                    ],
                    &[LogTag::Value(
                        "ability",
                        if old == EffectId::NONE {
                            LogArg::Empty
                        } else {
                            LogArg::Effect(EffectRef::Dex(old))
                        },
                    )],
                ));
            } else {
                let mut tags = [self.mutation_from(a.effect), LogTag::Bare("silent")];
                let len = if let Some(s) = Self::arg_mon(a.source) {
                    tags[1] = LogTag::Of(s);
                    2
                } else {
                    1
                };
                self.add(LogEntry::new(
                    "-ability",
                    &[
                        LogArg::Mon(pokemon),
                        if ability == EffectId::NONE {
                            LogArg::Empty
                        } else {
                            LogArg::Effect(EffectRef::Dex(ability))
                        },
                        if old == EffectId::NONE {
                            LogArg::Empty
                        } else {
                            LogArg::Effect(EffectRef::Dex(old))
                        },
                    ],
                    &tags[..len],
                ));
            }
        }
        if ability != EffectId::NONE && (!is_transform || old != ability) {
            self.mutation_single(
                EventId::Start,
                EffectRef::Dex(ability),
                cell,
                mon_arg(Some(pokemon)),
                Attribution {
                    effect: EffectRef::None,
                    ..a
                },
            );
        }
        if old == EffectId::NONE {
            Relay::NotFail
        } else {
            Relay::Effect(old)
        }
    }
    /// Set empty ability
    /// Ports `sim/pokemon.ts:1961-1963`. PRNG: none directly; dispatched events/callbacks may sort ties or draw.
    pub fn clear_ability(&mut self, pokemon: MonId) -> Relay {
        self.set_ability(pokemon, EffectId::NONE, Attribution::DEFAULT, false, false)
    }
    /// Match the ability first, then honor ignoringAbility
    /// Ports `sim/pokemon.ts:1952-1959`. PRNG: none.
    pub fn has_ability(&self, pokemon: MonId, abilities: &[EffectId]) -> bool {
        abilities.contains(&self.state.pokemon[pokemon.0 as usize].ability)
            && !self.ignoring_ability(pokemon)
    }
    /// Gastro Acid and Neutralizing Gas exceptions independent of Mold Breaker
    /// Ports `sim/pokemon.ts:858-877`. PRNG: none.
    pub fn ignoring_ability(&self, pokemon: MonId) -> bool {
        let mon = &self.state.pokemon[pokemon.0 as usize];
        if mon.flags & mon_flags::ACTIVE == 0 {
            return true;
        }
        let flags = if mon.ability == EffectId::NONE {
            0
        } else {
            dex::ABILITIES[(mon.ability.0 - dex::ABILITY_START) as usize].flags
        };
        // Transformation suppression deliberately precedes cantsuppress.
        if flags & dex::FLAG_NOTRANSFORM != 0 && mon.flags & mon_flags::TRANSFORMED != 0 {
            return true;
        }
        if flags & dex::FLAG_CANTSUPPRESS != 0 {
            return false;
        }
        if self.query_has_volatile(pokemon, "gastroacid") {
            return true;
        }
        if self.query_has_item(pokemon, "abilityshield")
            || query_effect_is(mon.ability, "neutralizinggas")
        {
            return false;
        }
        // Literal getAllActive() order/filter. Never recurse through hasAbility
        // on a gas holder; abilityState.ending is a shared truthiness flag.
        for side in &self.state.sides {
            for &active in &side.active {
                if active == MonId::NONE {
                    continue;
                }
                let holder = &self.state.pokemon[active.0 as usize];
                if holder.flags & mon_flags::FAINTED != 0 {
                    continue;
                }
                if query_effect_is(holder.ability, "neutralizinggas")
                    && !self.query_has_volatile(active, "gastroacid")
                    && holder.flags & mon_flags::TRANSFORMED == 0
                    && self.state.effects.cells[holder.ability_state.0 as usize].present
                        & present::ENDING
                        == 0
                    && !self.query_has_volatile(pokemon, "commanding")
                {
                    return true;
                }
            }
        }
        false
    }
}

// Static Dex keys let format-excluded source branches remain explicit without
// fabricating IDs for effects absent from this generated scope. NONE never
// matches a named ability/item/condition. These queries allocate and draw nothing.
fn query_effect_is(effect: EffectId, key: &str) -> bool {
    effect != EffectId::NONE && dex::effect(effect).key == key
}
impl<L: LogSink> Battle<L> {
    pub(crate) fn query_has_volatile(&self, pokemon: MonId, key: &str) -> bool {
        let list = &self.state.pokemon[pokemon.0 as usize].volatiles;
        list.cells[..list.len as usize]
            .iter()
            .any(|c| query_effect_is(self.state.effects.cells[c.0 as usize].id, key))
    }
    pub(crate) fn query_has_ability(&self, pokemon: MonId, key: &str) -> bool {
        query_effect_is(self.state.pokemon[pokemon.0 as usize].ability, key)
            && !self.ignoring_ability(pokemon)
    }
    pub(crate) fn query_has_item(&self, pokemon: MonId, key: &str) -> bool {
        query_effect_is(self.state.pokemon[pokemon.0 as usize].item, key)
            && !self.ignoring_item(pokemon)
    }
}
