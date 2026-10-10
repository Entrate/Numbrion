//! Suppression at dispatch time, never during handler collection.
//! Pinned sim/battle.ts:365-368,605-628,824-839,873-919. PRNG: none.
use crate::{
    Battle,
    dex::{self, EffectType},
    event::*,
    ids::*,
    log::LogSink,
    state::mon_flags,
};
impl<L: LogSink> Battle<L> {
    /// Active move suppression differs from Pokemon.ignoringAbility.
    /// Ports battle.ts:365-368 (fixed generation 9). PRNG: none.
    pub fn suppressing_ability(&self, target: Option<MonId>) -> bool {
        let user = self.scratch.active_pokemon;
        user != MonId::NONE
            && self.state.pokemon[user.0 as usize].flags & mon_flags::ACTIVE != 0
            && target != Some(user)
            && self.scratch.active_move != crate::actions::MoveHandle::NONE
            && self.active_move(self.scratch.active_move).runtime_flags
                & crate::state::scratch::move_runtime::IGNORE_ABILITY
                != 0
            && !target.is_some_and(|m| {
                // Ability Shield is outside the frozen scope. Never let the
                // absent optional ID match an empty item.
                const SHIELD: EffectId =
                    crate::effects::support::optional_id(dex::ITEMS_DATA, "abilityshield");
                SHIELD != EffectId::NONE && self.has_item(m, &[SHIELD])
            })
    }

    /// Independent singleEvent Start/End/TakeItem/SetAbility and weather exceptions.
    /// Ports battle.ts:605-628. PRNG: none.
    pub fn single_event_suppressed(
        &self,
        event: EventId,
        effect: EffectRef,
        target: EventArg,
    ) -> bool {
        let kind = self.event_effect_type(effect);
        let mon = Self::arg_mon(target);
        if kind == EffectType::Status
            && mon.is_some_and(|m| self.status_id(m) != self.event_effect_id(effect))
        {
            return true;
        }
        if event == EventId::SwitchIn
            && kind == EffectType::Ability
            && self.breakable_ability(effect)
            && self.suppressing_ability(mon)
        {
            return true;
        }
        if !matches!(
            event,
            EventId::Start | EventId::TakeItem | EventId::SetAbility
        ) && kind == EffectType::Item
            && mon.is_some_and(|m| self.ignoring_item(m))
        {
            return true;
        }
        if event != EventId::End
            && kind == EffectType::Ability
            && mon.is_some_and(|m| self.ignoring_ability(m))
        {
            return true;
        }
        kind == EffectType::Weather
            && !matches!(
                event,
                EventId::FieldStart | EventId::FieldResidual | EventId::FieldEnd
            )
            && self.suppressing_weather()
    }

    /// Re-evaluate a captured listener immediately before dispatch.
    /// Ports battle.ts:824-839,873-919. PRNG: none.
    pub fn run_event_suppressed(&self, event: EventId, listener: Listener) -> bool {
        let effect = listener.effect;
        let kind = self.event_effect_type(effect);
        let mon = (listener.holder.0 < 12).then_some(MonId(listener.holder.0));
        if kind == EffectType::Status
            && mon.is_some_and(|m| self.status_id(m) != self.event_effect_id(effect))
        {
            return true;
        }
        if kind == EffectType::Ability
            && self.breakable_ability(effect)
            && self.suppressing_ability(mon)
        {
            // The pinned custom-ability fallback is inside this breakable guard,
            // after its unconditional continue, and is unreachable.
            return true;
        }
        if !matches!(
            event,
            EventId::Start | EventId::SwitchIn | EventId::TakeItem
        ) && kind == EffectType::Item
            && mon.is_some_and(|m| self.ignoring_item(m))
        {
            return true;
        }
        if event != EventId::End
            && kind == EffectType::Ability
            && mon.is_some_and(|m| self.ignoring_ability(m))
        {
            return true;
        }
        (kind == EffectType::Weather || event == EventId::Weather)
            && !matches!(event, EventId::Residual | EventId::End)
            && self.suppressing_weather()
    }

    fn breakable_ability(&self, effect: EffectRef) -> bool {
        let id = self.event_effect_id(effect);
        id.kind() == Some(EffectKind::Ability)
            && dex::ABILITIES[(id.0 - dex::ABILITY_START) as usize].flags & dex::FLAG_BREAKABLE != 0
    }
}
#[cfg(test)]
#[path = "suppression_tests.rs"]
mod tests;
