//! Captured-listener discovery. Exact order from sim/battle.ts:1039-1251; no draws.
use crate::{
    Battle,
    dex::{self, EffectType, HookId, HookRel, HookValue},
    event::*,
    ids::*,
    log::LogSink,
    state::{CellId, mon_flags},
};
impl<L: LogSink> Battle<L> {
    pub fn find_event_handlers(
        &mut self,
        target: EventTarget,
        selector: HookSelector,
        source: Option<MonId>,
        buffer: u8,
    ) {
        if let EventTarget::Spread(ts) = target {
            for i in 0..ts.len as usize {
                let start = self.scratch.handlers[buffer as usize].len;
                self.find_event_handlers(
                    EventTarget::Single(EventArg::Holder(Holder::mon(ts.mons[i]))),
                    selector,
                    source,
                    buffer,
                );
                let b = &mut self.scratch.handlers[buffer as usize];
                for entry in &mut b.entries[start as usize..b.len as usize] {
                    let h = entry.as_mut().unwrap();
                    h.target_index = i as u8;
                    h.priority.index = i as u8;
                }
            }
            return;
        }
        let EventTarget::Single(arg) = target else {
            unreachable!()
        };
        let mut holder = Self::arg_holder(arg);
        let bubble = holder.0 == 12 || holder.0 == 13;
        let prefixed = !matches!(
            selector.event,
            EventId::BeforeTurn
                | EventId::Update
                | EventId::Weather
                | EventId::WeatherChange
                | EventId::TerrainChange
        );
        if holder.0 < 12
            && (self.state.pokemon[holder.0 as usize].flags & mon_flags::ACTIVE != 0
                || source.is_some_and(|m| {
                    self.state.pokemon[m.0 as usize].flags & mon_flags::ACTIVE != 0
                }))
        {
            let m = MonId(holder.0);
            self.find_pokemon_event_handlers(m, selector, buffer);
            if prefixed {
                for (side, rel) in [
                    (m.side(), HookRel::Ally),
                    (SideId(1 - m.side().0), HookRel::Foe),
                ] {
                    let (mons, len) = self.living_actives(side);
                    for a in mons[..len].iter().copied() {
                        // side.ts:390-403: allies()/foes() also drop hp-0 Pokemon that are
                        // queued to faint but not yet marked fainted.
                        if self.state.pokemon[a.0 as usize].hp == 0 {
                            continue;
                        }
                        self.find_pokemon_event_handlers(
                            a,
                            HookSelector { rel, ..selector },
                            buffer,
                        );
                        self.find_pokemon_event_handlers(
                            a,
                            HookSelector {
                                rel: HookRel::Any,
                                ..selector
                            },
                            buffer,
                        );
                    }
                }
            }
            holder = Holder::side(m.side());
        }
        if prefixed {
            if let Some(m) = source {
                self.find_pokemon_event_handlers(
                    m,
                    HookSelector {
                        rel: HookRel::Source,
                        ..selector
                    },
                    buffer,
                );
            }
        }
        if holder.0 == 12 || holder.0 == 13 {
            for side in [SideId(0), SideId(1)] {
                let own = Holder::side(side) == holder;
                let rel = if own { HookRel::On } else { HookRel::Foe };
                if bubble {
                    // findPokemonEventHandlers tolerates null; a fainted active still participates here.
                    for m in self.state.sides[side.0 as usize].active {
                        if m == MonId::NONE {
                            continue;
                        }
                        if own || prefixed {
                            self.find_pokemon_event_handlers(
                                m,
                                HookSelector { rel, ..selector },
                                buffer,
                            );
                        }
                        if prefixed {
                            self.find_pokemon_event_handlers(
                                m,
                                HookSelector {
                                    rel: HookRel::Any,
                                    ..selector
                                },
                                buffer,
                            );
                        }
                    }
                }
                if own || prefixed {
                    self.find_side_event_handlers(
                        side,
                        HookSelector { rel, ..selector },
                        None,
                        buffer,
                    );
                }
                if prefixed {
                    self.find_side_event_handlers(
                        side,
                        HookSelector {
                            rel: HookRel::Any,
                            ..selector
                        },
                        None,
                        buffer,
                    );
                }
            }
        }
        self.find_field_event_handlers(selector, None, buffer);
        self.find_battle_event_handlers(selector, None, buffer);
    }
    fn collect_cell(
        &mut self,
        effect: EffectRef,
        cell: CellId,
        holder: Holder,
        selector: HookSelector,
        end: EndHandler,
        buffer: u8,
    ) {
        if cell == CellId::NONE {
            return;
        }
        let hook = self.get_callback(holder, effect, selector.event, selector.rel);
        let duration = self.state.effects.cells[cell.0 as usize].duration;
        if hook.is_none()
            && !(matches!(selector.mode, CollectMode::Duration) && duration != -1 && duration != 0)
        {
            return;
        }
        let state = Some(self.state.effects.capture(cell));
        let listener = Listener {
            effect,
            selector,
            hook,
            state,
            holder,
            priority: Priority::default(),
            target_index: 255,
            end,
        };
        self.push_listener(buffer, self.resolve_priority(listener));
    }
    pub fn find_pokemon_event_handlers(
        &mut self,
        pokemon: MonId,
        selector: HookSelector,
        buffer: u8,
    ) {
        if pokemon == MonId::NONE {
            return;
        }
        let p = self.state.pokemon[pokemon.0 as usize];
        let holder = Holder::mon(pokemon);
        self.collect_cell(
            EffectRef::Dex(self.status_id(pokemon)),
            p.status_state,
            holder,
            selector,
            EndHandler::Status(pokemon),
            buffer,
        );
        for c in p.volatiles.as_slice().iter().copied() {
            let id = self.state.effects.cells[c.0 as usize].id;
            self.collect_cell(
                EffectRef::Dex(id),
                c,
                holder,
                selector,
                EndHandler::Volatile(pokemon, id),
                buffer,
            );
        }
        self.collect_cell(
            EffectRef::Dex(p.ability),
            p.ability_state,
            holder,
            selector,
            EndHandler::Ability(pokemon),
            buffer,
        );
        self.collect_cell(
            EffectRef::Dex(p.item),
            p.item_state,
            holder,
            selector,
            EndHandler::Item(pokemon),
            buffer,
        );
        // Species durations do not participate.
        self.collect_cell(
            EffectRef::Dex(p.base_species),
            p.species_state,
            holder,
            HookSelector {
                mode: CollectMode::Callback,
                ..selector
            },
            EndHandler::None,
            buffer,
        );
        if p.position < 2 {
            let slots =
                self.state.sides[pokemon.side().0 as usize].slot_conditions[p.position as usize];
            for c in slots.as_slice().iter().copied() {
                let id = self.state.effects.cells[c.0 as usize].id;
                self.collect_cell(
                    EffectRef::Dex(id),
                    c,
                    holder,
                    selector,
                    EndHandler::Slot(SlotId::new(pokemon.side(), p.position), id),
                    buffer,
                );
            }
        }
    }
    pub fn find_battle_event_handlers(
        &mut self,
        selector: HookSelector,
        custom_holder: Option<MonId>,
        buffer: u8,
    ) {
        self.collect_cell(
            EffectRef::Synthetic(SyntheticEffect::Format),
            self.state.format_state,
            custom_holder.map_or(Holder::BATTLE, Holder::mon),
            selector,
            EndHandler::None,
            buffer,
        );
        // No onEvent registrations exist in the pinned gen9randomdoublesbattle closure.
    }
    pub fn find_field_event_handlers(
        &mut self,
        selector: HookSelector,
        custom_holder: Option<MonId>,
        buffer: u8,
    ) {
        let holder = custom_holder.map_or(Holder::FIELD, Holder::mon);
        let field = self.state.field;
        for c in field.pseudo_weather.as_slice().iter().copied() {
            let id = self.state.effects.cells[c.0 as usize].id;
            let end = if custom_holder.is_some() {
                EndHandler::None
            } else {
                EndHandler::PseudoWeather(id)
            };
            self.collect_cell(EffectRef::Dex(id), c, holder, selector, end, buffer);
        }
        for (c, end) in [
            (field.weather, EndHandler::Weather),
            (field.terrain, EndHandler::Terrain),
        ] {
            let id = self.state.effects.cells[c.0 as usize].id;
            self.collect_cell(
                EffectRef::Dex(id),
                c,
                holder,
                selector,
                if custom_holder.is_some() {
                    EndHandler::None
                } else {
                    end
                },
                buffer,
            );
        }
    }
    pub fn find_side_event_handlers(
        &mut self,
        side: SideId,
        selector: HookSelector,
        custom_holder: Option<MonId>,
        buffer: u8,
    ) {
        let list = self.state.sides[side.0 as usize].conditions;
        for c in list.as_slice().iter().copied() {
            let id = self.state.effects.cells[c.0 as usize].id;
            self.collect_cell(
                EffectRef::Dex(id),
                c,
                custom_holder.map_or(Holder::side(side), Holder::mon),
                selector,
                if custom_holder.is_some() {
                    EndHandler::None
                } else {
                    EndHandler::Side(side, id)
                },
                buffer,
            );
        }
    }
    pub fn get_callback(
        &self,
        holder: Holder,
        effect: EffectRef,
        event: EventId,
        rel: HookRel,
    ) -> Option<HookId> {
        let defined = |h: HookId| !matches!(dex::HOOKS[h.0 as usize].value, HookValue::Absent);
        if let Some(h) = self.event_hook(effect, event, rel).filter(|h| defined(*h)) {
            return Some(h);
        }
        if holder.0 < 12
            && event == EventId::SwitchIn
            && rel == HookRel::On
            && self
                .event_hook(effect, event, HookRel::Any)
                .filter(|h| defined(*h))
                .is_none()
            && matches!(
                self.event_effect_type(effect),
                EffectType::Ability | EffectType::Item
            )
        {
            return self
                .event_hook(effect, EventId::Start, HookRel::On)
                .filter(|h| defined(*h));
        }
        None
    }
    pub fn push_listener(&mut self, buffer: u8, mut listener: Listener) {
        let b = &mut self.scratch.handlers[buffer as usize];
        assert!(
            (b.len as usize) < HANDLER_CAPACITY,
            "handler capacity exceeded"
        );
        if let Some(r) = listener.state {
            let pin = self.state.effects.pin(r.cell);
            assert_eq!(pin, r);
            listener.state = Some(pin);
        }
        b.entries[b.len as usize] = Some(listener);
        b.len += 1;
    }
    pub fn release_handlers(&mut self, buffer: u8) {
        let b = &mut self.scratch.handlers[buffer as usize];
        for e in &mut b.entries[..b.len as usize] {
            if let Some(h) = e.take() {
                if let Some(r) = h.state {
                    self.state.effects.unpin(r);
                }
            }
        }
        b.len = 0;
    }
}
