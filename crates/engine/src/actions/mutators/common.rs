//! Internal helpers shared by the pinned mutation ports. No event or PRNG is hidden here.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookRel},
    event::{CallArgs, EffectRef, EventArg, Relay, RunEventOptions},
    ids::*,
    log::LogSink,
    state::{CellId, mon_flags, present},
};
// Reserved snapshot bits, documented in the owner report for lifecycle resets.
pub const USED_ITEM_THIS_TURN: u32 = 1 << 22;
pub const ATE_BERRY: u32 = 1 << 23;
pub(crate) fn mon_arg(mon: Option<MonId>) -> EventArg {
    mon.map_or(EventArg::Null, |m| EventArg::Holder(Holder::mon(m)))
}
pub(crate) fn number(r: Relay) -> f64 {
    match r {
        Relay::Number(n) => n,
        Relay::Bool(true) => 1.,
        _ => 0.,
    }
}
impl<L: LogSink> Battle<L> {
    pub(crate) fn mutation_frame(&self) -> Option<crate::event::EventFrame> {
        (self.scratch.current_frame != 255).then(|| {
            self.scratch.frames[self.scratch.current_frame as usize].expect("missing event")
        })
    }
    // Pokemon/battle mutators use event.source; field/item methods use event.target.
    pub(crate) fn mutation_attribution(
        &self,
        mut a: Attribution,
        from_target: bool,
        default_effect: bool,
    ) -> Attribution {
        if let Some(f) = self.mutation_frame() {
            if Self::arg_mon(a.source).is_none() {
                a.source = if from_target { f.target } else { f.source };
            }
            if default_effect && a.effect == EffectRef::None {
                a.effect = self.scratch.current_effect;
            }
        } else if default_effect && from_target && a.effect == EffectRef::None {
            a.effect = self.scratch.current_effect;
        }
        a
    }
    pub(crate) fn mutation_event(
        &mut self,
        event: EventId,
        target: EventArg,
        a: Attribution,
        relay: Relay,
    ) -> Relay {
        self.run_event(
            event,
            target,
            a.source,
            a.effect,
            relay,
            RunEventOptions::default(),
        )
    }
    pub(crate) fn mutation_single(
        &mut self,
        event: EventId,
        effect: EffectRef,
        cell: CellId,
        target: EventArg,
        a: Attribution,
    ) -> Relay {
        let state = (cell != CellId::NONE).then(|| self.state.effects.capture(cell));
        self.single_event(
            event,
            effect,
            state,
            target,
            a.source,
            a.effect,
            Relay::Undefined,
            None,
        )
    }
    pub(crate) fn mutation_cell(&mut self, owner: Holder, target: Holder, id: EffectId) -> CellId {
        let order = if id != EffectId::NONE
            && target != Holder::NONE
            && (target.0 >= 12
                || self.state.pokemon[target.0 as usize].flags & mon_flags::ACTIVE != 0)
        {
            let n = self.state.effect_order;
            self.state.effect_order += 1;
            n
        } else {
            0
        };
        self.state.effects.alloc(owner, target, id, order)
    }
    pub(crate) fn condition_id(&self, id: EffectId) -> EffectId {
        if id == EffectId::NONE {
            return id;
        }
        dex::lookup(EffectKind::Condition, dex::effect(id).key)
            .map_or(dex::canonical_effect(id), dex::canonical_effect)
    }
    pub(crate) fn condition_ref(&self, id: EffectId) -> EffectRef {
        let id = self.condition_id(id);
        match id.kind() {
            Some(EffectKind::Move) => EffectRef::MoveCondition(id),
            Some(EffectKind::Ability) => EffectRef::AbilityCondition(id),
            Some(EffectKind::Item) => EffectRef::ItemCondition(id),
            _ => EffectRef::Dex(id),
        }
    }
    pub(crate) fn condition_source(
        &mut self,
        cell: CellId,
        a: Attribution,
        slot: bool,
        effect: bool,
    ) {
        let source = Self::arg_mon(a.source);
        let source_slot =
            source.map(|m| SlotId::new(m.side(), self.state.pokemon[m.0 as usize].position));
        let source_effect = self.freeze_effect(a.effect);
        let c = &mut self.state.effects.cells[cell.0 as usize];
        if let Some(source) = source {
            c.source = source;
            c.present |= present::SOURCE;
        }
        if slot {
            if let Some(s) = source_slot {
                c.source_slot = s;
                c.present |= present::SOURCE_SLOT;
            }
        }
        if effect && a.effect != EffectRef::None {
            c.source_effect = EffectId(source_effect.0);
            c.present |= present::SOURCE_EFFECT;
        }
    }
    pub(crate) fn condition_duration(
        &mut self,
        cell: CellId,
        effect: EffectRef,
        target: EventArg,
        a: Attribution,
    ) {
        let id = self.event_effect_id(effect);
        if id != EffectId::NONE {
            if let Some(dex::DataValue::Number(n)) = dex::effect(id).data.get(dex::FIELD_DURATION) {
                if n != 0. {
                    self.state.effects.cells[cell.0 as usize].duration = n as i16;
                }
            }
        }
        if let Some(h) = self.event_hook(effect, EventId::DurationCallback, HookRel::Direct) {
            let r = self.call_hook(
                h,
                CallArgs {
                    values: [
                        target,
                        a.source,
                        EventArg::Effect(a.effect),
                        EventArg::Undefined,
                    ],
                    len: 3,
                },
            );
            self.state.effects.cells[cell.0 as usize].duration = number(r) as i16;
        }
    }
    pub(crate) fn release_cell(&mut self, cell: CellId) {
        if cell != CellId::NONE {
            self.state.effects.release(cell);
        }
    }
    pub(crate) fn effect_status(&self, effect: EffectRef) -> EffectId {
        if let Some(s) = self.synthetic_status(effect) {
            return s;
        }
        match effect {
            EffectRef::ActiveMove(i) => self.scratch.moves[i as usize].unwrap().effects.status,
            EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
                dex::move_data(id).effects.status
            }
            _ => EffectId::NONE,
        }
    }
}
