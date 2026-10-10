//! Captured-listener discovery. Exact order from sim/battle.ts:1039-1251; no draws.
use crate::{
    Battle,
    dex::{self, EffectType, HookId, HookRel, HookValue},
    event::*,
    ids::*,
    log::LogSink,
    state::{CellId, mon_flags},
};
/// One discovery pass's callback relations per holder (Pokemon 0-11, sides
/// 12-13, field 14) for a single event. Collection runs no callbacks and changes
/// no effect references, so a pass may reuse them; nothing persists across
/// passes or needs invalidation. A clear bit proves that collection is empty.
struct PassRelations {
    event: EventId,
    known: u16,
    masks: [u8; 15],
}
impl<L: LogSink> Battle<L> {
    pub fn find_event_handlers(
        &mut self,
        target: EventTarget,
        selector: HookSelector,
        source: Option<MonId>,
        buffer: u8,
    ) {
        if matches!(selector.mode, CollectMode::Callback)
            && dex::callback_relations(selector.event) == 0
        {
            return;
        }
        let mut relations = PassRelations {
            event: selector.event,
            known: 0,
            masks: [0; 15],
        };
        self.find_event_handlers_in(target, selector, source, buffer, &mut relations);
    }
    fn find_event_handlers_in(
        &mut self,
        target: EventTarget,
        selector: HookSelector,
        source: Option<MonId>,
        buffer: u8,
        relations: &mut PassRelations,
    ) {
        if let EventTarget::Spread(ts) = target {
            for i in 0..ts.len as usize {
                let start = self.scratch.handlers[buffer as usize].len;
                self.find_event_handlers_in(
                    EventTarget::Single(EventArg::Holder(Holder::mon(ts.mons[i]))),
                    selector,
                    source,
                    buffer,
                    relations,
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
            self.find_pokemon_event_handlers_in(relations, m, selector, buffer);
            if prefixed {
                for (side, rel) in [
                    (m.side(), HookRel::Ally),
                    (SideId(1 - m.side().0), HookRel::Foe),
                ] {
                    if matches!(selector.mode, CollectMode::Callback)
                        && !dex::has_callback(selector.event, rel)
                        && !dex::has_callback(selector.event, HookRel::Any)
                    {
                        continue;
                    }
                    let (mons, len) = self.living_actives(side);
                    for a in mons[..len].iter().copied() {
                        // side.ts:390-403: allies()/foes() also drop hp-0 Pokemon that are
                        // queued to faint but not yet marked fainted.
                        if self.state.pokemon[a.0 as usize].hp == 0 {
                            continue;
                        }
                        self.find_pokemon_event_handlers_in(
                            relations,
                            a,
                            HookSelector { rel, ..selector },
                            buffer,
                        );
                        self.find_pokemon_event_handlers_in(
                            relations,
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
                self.find_pokemon_event_handlers_in(
                    relations,
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
                            self.find_pokemon_event_handlers_in(
                                relations,
                                m,
                                HookSelector { rel, ..selector },
                                buffer,
                            );
                        }
                        if prefixed {
                            self.find_pokemon_event_handlers_in(
                                relations,
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
                let side_holder = 12 + side.0 as usize;
                if (own || prefixed) && self.pass_may_collect(relations, side_holder, selector, rel)
                {
                    self.find_side_event_handlers(
                        side,
                        HookSelector { rel, ..selector },
                        None,
                        buffer,
                    );
                }
                if prefixed && self.pass_may_collect(relations, side_holder, selector, HookRel::Any)
                {
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
        if self.pass_may_collect(relations, 14, selector, selector.rel) {
            self.find_field_event_handlers(selector, None, buffer);
        }
        self.find_battle_event_handlers(selector, None, buffer);
    }
    /// False only when Callback collection for this holder and relation is
    /// provably empty. Duration collection is never pruned.
    fn pass_may_collect(
        &self,
        relations: &mut PassRelations,
        holder: usize,
        selector: HookSelector,
        rel: HookRel,
    ) -> bool {
        if !matches!(selector.mode, CollectMode::Callback) {
            return true;
        }
        // Globally absent relations need no holder mask (as the collectors check).
        if !dex::has_callback(selector.event, rel) {
            return false;
        }
        debug_assert_eq!(relations.event, selector.event);
        if relations.known & (1 << holder) == 0 {
            relations.masks[holder] = match holder {
                0..12 => self.pokemon_callback_relations(MonId(holder as u8), selector.event),
                12 | 13 => {
                    let conditions = self.state.sides[holder - 12].conditions;
                    self.cells_callback_relations(conditions.as_slice(), selector.event)
                }
                _ => {
                    let field = &self.state.field;
                    self.cells_callback_relations(field.pseudo_weather.as_slice(), selector.event)
                        | self.cells_callback_relations(
                            &[field.weather, field.terrain],
                            selector.event,
                        )
                }
            };
            relations.known |= 1 << holder;
        }
        relations.masks[holder] & (1 << rel as usize) != 0
    }
    fn cells_callback_relations(&self, cells: &[CellId], event: EventId) -> u8 {
        let mut mask = 0;
        for &c in cells {
            if c != CellId::NONE {
                mask |= dex::effect_callback_relations(
                    self.state.effects.cells[c.0 as usize].id,
                    event,
                );
            }
        }
        mask
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
        if matches!(selector.mode, CollectMode::Callback) {
            if let EffectRef::Dex(id) = effect {
                if !dex::effect_has_callback(id, selector.event, selector.rel) {
                    return;
                }
            }
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
        if pokemon == MonId::NONE
            || (matches!(selector.mode, CollectMode::Callback)
                && (!dex::has_callback(selector.event, selector.rel)
                    || !self.pokemon_has_callback(pokemon, selector)))
        {
            return;
        }
        self.collect_pokemon_cells(pokemon, selector, buffer);
    }
    /// `find_pokemon_event_handlers` with this pass's cached relation masks.
    fn find_pokemon_event_handlers_in(
        &mut self,
        relations: &mut PassRelations,
        pokemon: MonId,
        selector: HookSelector,
        buffer: u8,
    ) {
        if pokemon == MonId::NONE
            || (matches!(selector.mode, CollectMode::Callback)
                && !dex::has_callback(selector.event, selector.rel))
            || !self.pass_may_collect(relations, pokemon.0 as usize, selector, selector.rel)
        {
            return;
        }
        self.collect_pokemon_cells(pokemon, selector, buffer);
    }
    fn collect_pokemon_cells(&mut self, pokemon: MonId, selector: HookSelector, buffer: u8) {
        // Snapshot only the fields read below, not the whole Pokemon.
        let p = &self.state.pokemon[pokemon.0 as usize];
        let (volatiles, position) = (p.volatiles, p.position);
        let (status_state, ability, ability_state) = (p.status_state, p.ability, p.ability_state);
        let (item, item_state) = (p.item, p.item_state);
        let (base_species, species_state) = (p.base_species, p.species_state);
        let holder = Holder::mon(pokemon);
        self.collect_cell(
            EffectRef::Dex(self.status_id(pokemon)),
            status_state,
            holder,
            selector,
            EndHandler::Status(pokemon),
            buffer,
        );
        for c in volatiles.as_slice().iter().copied() {
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
            EffectRef::Dex(ability),
            ability_state,
            holder,
            selector,
            EndHandler::Ability(pokemon),
            buffer,
        );
        self.collect_cell(
            EffectRef::Dex(item),
            item_state,
            holder,
            selector,
            EndHandler::Item(pokemon),
            buffer,
        );
        // Species durations do not participate.
        self.collect_cell(
            EffectRef::Dex(base_species),
            species_state,
            holder,
            HookSelector {
                mode: CollectMode::Callback,
                ..selector
            },
            EndHandler::None,
            buffer,
        );
        if position < 2 {
            let slots =
                self.state.sides[pokemon.side().0 as usize].slot_conditions[position as usize];
            for c in slots.as_slice().iter().copied() {
                let id = self.state.effects.cells[c.0 as usize].id;
                self.collect_cell(
                    EffectRef::Dex(id),
                    c,
                    holder,
                    selector,
                    EndHandler::Slot(SlotId::new(pokemon.side(), position), id),
                    buffer,
                );
            }
        }
    }
    /// Union of `effect_has_callback` relation bits over the Pokemon's current
    /// effects; the same absence proof as `pokemon_has_callback`, for every relation.
    fn pokemon_callback_relations(&self, pokemon: MonId, event: EventId) -> u8 {
        let p = &self.state.pokemon[pokemon.0 as usize];
        let rels = |id| dex::effect_callback_relations(id, event);
        let cell_rels = |c: &CellId| rels(self.state.effects.cells[c.0 as usize].id);
        let mut mask =
            rels(p.ability) | rels(p.item) | rels(p.base_species) | rels(self.status_id(pokemon));
        for c in p.volatiles.as_slice() {
            mask |= cell_rels(c);
        }
        if p.position < 2 {
            let slots = &self.state.sides[pokemon.side().0 as usize].slot_conditions;
            for c in slots[p.position as usize].as_slice() {
                mask |= cell_rels(c);
            }
        }
        mask
    }
    /// Recompute from current effect references, so direct state edits, nested
    /// installs/removals and snapshot restoration require no cache invalidation.
    /// Only proves absence; collection still captures and pins every listener.
    fn pokemon_has_callback(&self, pokemon: MonId, selector: HookSelector) -> bool {
        let p = &self.state.pokemon[pokemon.0 as usize];
        let has = |id| dex::effect_has_callback(id, selector.event, selector.rel);
        if has(p.ability) || has(p.item) || has(p.base_species) || has(self.status_id(pokemon)) {
            return true;
        }
        let cell_has = |c: &CellId| has(self.state.effects.cells[c.0 as usize].id);
        if p.volatiles.as_slice().iter().any(cell_has) {
            return true;
        }
        p.position < 2
            && self.state.sides[pokemon.side().0 as usize].slot_conditions[p.position as usize]
                .as_slice()
                .iter()
                .any(cell_has)
    }
    pub fn find_battle_event_handlers(
        &mut self,
        selector: HookSelector,
        custom_holder: Option<MonId>,
        buffer: u8,
    ) {
        // The synthetic Format has no hooks here (event_hook resolves it to no
        // effect, and the SwitchIn fallback needs an ability or item), so
        // Callback collection is always empty; only durations can be found.
        if matches!(selector.mode, CollectMode::Callback) {
            return;
        }
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
        if matches!(selector.mode, CollectMode::Callback)
            && !dex::has_callback(selector.event, selector.rel)
        {
            return;
        }
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
        if matches!(selector.mode, CollectMode::Callback)
            && !dex::has_callback(selector.event, selector.rel)
        {
            return;
        }
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
        b.entries.push(Some(listener));
        b.len += 1;
    }
    pub fn release_handlers(&mut self, buffer: u8) {
        let b = &mut self.scratch.handlers[buffer as usize];
        for h in b.entries.drain(..).flatten() {
            if let Some(r) = h.state {
                self.state.effects.unpin(r);
            }
        }
        b.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::NoLog;

    #[test]
    fn format_callback_collection_is_always_empty() {
        let packed = "Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric";
        let battle: Battle<NoLog> = Battle::new([1, 2, 3, 4], packed, packed).unwrap();
        let format = EffectRef::Synthetic(SyntheticEffect::Format);
        for event in dex::HOOKS.iter().map(|h| h.event) {
            for rel in [
                HookRel::On,
                HookRel::Source,
                HookRel::Ally,
                HookRel::Foe,
                HookRel::Any,
                HookRel::Direct,
            ] {
                for holder in [Holder::BATTLE, Holder::mon(MonId(0))] {
                    assert_eq!(battle.get_callback(holder, format, event, rel), None);
                }
            }
        }
    }
}
