//! Derived at compile time from the generated declarative property trees. Hot
//! battle reads of a rare field use these per-id tables instead of scanning an
//! effect's properties; tests compare every id with `DataValue::get`.
use super::*;

/// `DataValue::get` (first matching property) for constant evaluation.
const fn field(value: DataValue, key: FieldId) -> Option<DataValue> {
    if let DataValue::Object(props) = value {
        let mut i = 0;
        while i < props.len() {
            if props[i].key.0 == key.0 {
                return Some(props[i].value);
            }
            i += 1;
        }
    }
    None
}

static PRIMAL_ORB: [bool; EFFECT_COUNT] = {
    let mut table = [false; EFFECT_COUNT];
    let mut id = 1;
    while id < EFFECT_COUNT {
        let value = field(effect(EffectId(id as u16)).data, FIELD_ISPRIMALORB);
        table[id] = matches!(value, Some(DataValue::Bool(true)));
        id += 1;
    }
    table
};

/// `id != NONE && effect(id).data.get(FIELD_ISPRIMALORB) == Some(Bool(true))`.
#[inline]
pub fn is_primal_orb(id: EffectId) -> bool {
    PRIMAL_ORB[id.0 as usize]
}

// A numeric `duration` of 0 never applies, so it is None like an absent field.
static DURATIONS: [Option<i16>; EFFECT_COUNT] = {
    let mut table = [None; EFFECT_COUNT];
    let mut id = 1;
    while id < EFFECT_COUNT {
        if let Some(DataValue::Number(n)) = field(effect(EffectId(id as u16)).data, FIELD_DURATION)
        {
            if n != 0. {
                table[id] = Some(n as i16);
            }
        }
        id += 1;
    }
    table
};

/// The declarative `duration` an install applies: `Some(n as i16)` for a numeric,
/// nonzero `effect(id).data.get(FIELD_DURATION)` of a non-NONE id, else None.
#[inline]
pub fn data_duration(id: EffectId) -> Option<i16> {
    DURATIONS[id.0 as usize]
}

static SELF_CHANCES: [Option<u16>; EFFECT_COUNT] = {
    let mut table = [None; EFFECT_COUNT];
    let mut id = 1;
    while id < EFFECT_COUNT {
        if let Some(own) = field(effect(EffectId(id as u16)).data, FIELD_SELF) {
            if let Some(DataValue::Number(n)) = field(own, FIELD_CHANCE) {
                table[id] = Some(n as u16);
            }
        }
        id += 1;
    }
    table
};

/// `effect(id).data.get(FIELD_SELF)?.get(FIELD_CHANCE)?` as a number, `as u16`.
#[inline]
pub fn self_chance(id: EffectId) -> Option<u16> {
    SELF_CHANCES[id.0 as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_tables_match_property_reads_for_every_id() {
        assert!(!is_primal_orb(EffectId::NONE));
        assert_eq!(data_duration(EffectId::NONE), None);
        for id in 1..EFFECT_COUNT as u16 {
            let id = EffectId(id);
            let data = effect(id).data;
            assert_eq!(
                is_primal_orb(id),
                data.get(FIELD_ISPRIMALORB) == Some(DataValue::Bool(true)),
                "{id:?}"
            );
            let duration = match data.get(FIELD_DURATION) {
                Some(DataValue::Number(n)) if n != 0. => Some(n as i16),
                _ => None,
            };
            assert_eq!(data_duration(id), duration, "{id:?}");
            let chance = match data.get(FIELD_SELF).and_then(|s| s.get(FIELD_CHANCE)) {
                Some(DataValue::Number(n)) => Some(n as u16),
                _ => None,
            };
            assert_eq!(self_chance(id), chance, "{id:?}");
        }
    }
}
