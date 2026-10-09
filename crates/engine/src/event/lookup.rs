//! Allocation-free effect views used by dispatch. battle.ts:1022; dex-conditions.ts:43-130.
use crate::{
    Battle,
    dex::{self, *},
    event::*,
    ids::*,
    log::LogSink,
    state::{Status, mon_flags},
};
impl<L: LogSink> Battle<L> {
    pub fn event_effect_id(&self, effect: EffectRef) -> EffectId {
        match effect {
            EffectRef::Dex(id)
            | EffectRef::SpeciesCondition(id)
            | EffectRef::MoveCondition(id)
            | EffectRef::AbilityCondition(id)
            | EffectRef::ItemCondition(id) => dex::canonical_effect(id),
            EffectRef::ActiveMove(i) => self.scratch.moves[i as usize].expect("released move").id,
            _ => EffectId::NONE,
        }
    }
    pub fn event_effect_type(&self, effect: EffectRef) -> EffectType {
        match effect {
            EffectRef::SpeciesCondition(id) => SPECIES_CONDITION_VIEWS
                .iter()
                .find(|v| v.species == id)
                .map_or(EffectType::Condition, |v| v.effect_type),
            EffectRef::Synthetic(SyntheticEffect::Format) => EffectType::Format,
            EffectRef::Synthetic(_) => EffectType::Move,
            EffectRef::ActiveMove(_) => EffectType::Move,
            _ => MANIFESTS[self.event_effect_id(effect).0 as usize].effect_type,
        }
    }
    /// Exact key lookup. Absent ordering-only entries may be requested separately by the sorter.
    pub fn event_hook(&self, effect: EffectRef, event: EventId, rel: HookRel) -> Option<HookId> {
        let id = self.event_effect_id(effect);
        if id == EffectId::NONE {
            return None;
        }
        if let EffectRef::SpeciesCondition(species) = effect {
            return SPECIES_CONDITION_VIEWS
                .iter()
                .find(|v| v.species == species)
                .and_then(|v| {
                    v.hooks.iter().copied().find(|h| {
                        let h = &HOOKS[h.0 as usize];
                        h.event == event && h.rel == rel
                    })
                });
        }
        let manifest = &MANIFESTS[id.0 as usize];
        if !manifest.has_event(event) {
            return None;
        }
        manifest
            .hooks()
            .iter()
            .enumerate()
            .find(|(_, h)| h.event == event && h.rel == rel && h.site.is_empty())
            .map(|(i, _)| HookId(manifest.hooks_start + i as u16))
    }
    pub(crate) fn arg_mon(arg: EventArg) -> Option<MonId> {
        match arg {
            EventArg::Holder(h) if h.0 < 12 => Some(MonId(h.0)),
            EventArg::Relay(Relay::Pokemon(m)) => Some(m),
            _ => None,
        }
    }
    pub(crate) fn arg_holder(arg: EventArg) -> Holder {
        match arg {
            EventArg::Holder(h) => h,
            EventArg::Relay(Relay::Pokemon(m)) => Holder::mon(m),
            _ => Holder::NONE,
        }
    }
    pub(crate) fn status_id(&self, m: MonId) -> EffectId {
        match self.state.pokemon[m.0 as usize].status {
            Status::None => EffectId::NONE,
            Status::Burn => CONDITION_BRN,
            Status::Paralysis => CONDITION_PAR,
            Status::Sleep => CONDITION_SLP,
            Status::Freeze => CONDITION_FRZ,
            Status::Poison => CONDITION_PSN,
            Status::Toxic => CONDITION_TOX,
            Status::Fainted => EffectId::NONE,
        }
    }
    pub(crate) fn living_actives(&self, side: SideId) -> ([MonId; 2], usize) {
        let mut mons = [MonId::NONE; 2];
        let mut len = 0;
        for m in self.state.sides[side.0 as usize].active {
            if m != MonId::NONE && self.state.pokemon[m.0 as usize].flags & mon_flags::FAINTED == 0
            {
                mons[len] = m;
                len += 1;
            }
        }
        (mons, len)
    }
}
