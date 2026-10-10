//! Derived at compile time from the generated effect keys. Battle code compares
//! typed ids only; every key-to-id conversion it needs is resolved here, before
//! dispatch, and is checked exhaustively against the string forms it replaces.
use super::*;

const fn same_key(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Every raw id whose generated key equals one literal, across all kinds and
/// without alias canonicalization: `contains(id)` is exactly
/// `id != EffectId::NONE && dex::effect(id).key == key`. A key outside the
/// generated scope yields the empty set, so it never matches (and never
/// fabricates an id). Build with `key_ids!` to force compile-time evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyIds {
    ids: [EffectId; KeyIds::CAPACITY],
    len: u8,
}
impl KeyIds {
    /// One slot per effect kind; keys are unique within a generated table.
    const CAPACITY: usize = 6;
    pub const fn of(key: &str) -> Self {
        let mut set = Self {
            ids: [EffectId::NONE; Self::CAPACITY],
            len: 0,
        };
        let mut id = 1;
        while id < EFFECT_COUNT {
            if same_key(effect(EffectId(id as u16)).key, key) {
                set.ids[set.len as usize] = EffectId(id as u16);
                set.len += 1;
            }
            id += 1;
        }
        set
    }
    #[inline(always)]
    pub fn contains(self, id: EffectId) -> bool {
        self.ids[..self.len as usize].contains(&id)
    }
    pub fn ids(&self) -> &[EffectId] {
        &self.ids[..self.len as usize]
    }
}

/// `dex::KeyIds::of(key)`, evaluated at compile time.
macro_rules! key_ids {
    ($key:expr) => {
        const { $crate::dex::KeyIds::of($key) }
    };
}
pub(crate) use key_ids;

const fn canonical_const(id: EffectId) -> EffectId {
    let mut i = 0;
    while i < CONDITION_RULE_ALIASES.len() {
        if CONDITION_RULE_ALIASES[i].0.0 == id.0 {
            return CONDITION_RULE_ALIASES[i].1;
        }
        i += 1;
    }
    id
}

/// `lookup(EffectKind::Condition, effect(id).key)` per raw id (NONE if absent).
/// Generated keys are already normalized, so the exact comparison is the lookup.
static SAME_KEY_CONDITION: [EffectId; EFFECT_COUNT] = {
    let mut table = [EffectId::NONE; EFFECT_COUNT];
    let mut id = 1;
    while id < EFFECT_COUNT {
        let key = effect(EffectId(id as u16)).key;
        let mut c = 0;
        while c < CONDITIONS_DATA.len() {
            if same_key(CONDITIONS_DATA[c].key, key) {
                table[id] = CONDITIONS_DATA[c].id;
                break;
            }
            c += 1;
        }
        id += 1;
    }
    table
};

/// The runtime condition identity of an effect id: its same-key condition
/// record when one exists, else the id itself, then the single alias step.
static CONDITION_IDS: [EffectId; EFFECT_COUNT] = {
    let mut table = [EffectId::NONE; EFFECT_COUNT];
    let mut id = 1;
    while id < EFFECT_COUNT {
        let same = SAME_KEY_CONDITION[id];
        table[id] = canonical_const(if same.0 == 0 {
            EffectId(id as u16)
        } else {
            same
        });
        id += 1;
    }
    table
};

/// `dex::lookup(EffectKind::Condition, dex::effect(id).key)` without the string search.
#[inline]
pub fn same_key_condition(id: EffectId) -> Option<EffectId> {
    let c = SAME_KEY_CONDITION[id.0 as usize];
    (c != EffectId::NONE).then_some(c)
}

/// Condition identity for `addVolatile(id)`-style installs; NONE stays NONE.
#[inline]
pub fn condition_id(id: EffectId) -> EffectId {
    CONDITION_IDS[id.0 as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [EffectKind; 6] = [
        EffectKind::Species,
        EffectKind::Move,
        EffectKind::Ability,
        EffectKind::Item,
        EffectKind::Condition,
        EffectKind::Rule,
    ];

    /// The runtime implementation `condition_id` replaced.
    fn condition_id_by_lookup(id: EffectId) -> EffectId {
        if id == EffectId::NONE {
            return id;
        }
        lookup(EffectKind::Condition, effect(id).key).map_or(canonical_effect(id), canonical_effect)
    }

    #[test]
    fn condition_tables_match_string_lookup_for_every_id() {
        assert_eq!(condition_id(EffectId::NONE), EffectId::NONE);
        assert_eq!(same_key_condition(EffectId::NONE), None);
        for id in 1..EFFECT_COUNT as u16 {
            let id = EffectId(id);
            assert_eq!(effect(id).id, id);
            assert_eq!(condition_id(id), condition_id_by_lookup(id), "{id:?}");
            assert_eq!(
                same_key_condition(id),
                lookup(EffectKind::Condition, effect(id).key),
                "{id:?}"
            );
        }
    }

    #[test]
    fn key_sets_match_string_comparison_for_every_key_and_id() {
        let mut keys: Vec<&str> = KINDS
            .iter()
            .flat_map(|&kind| table(kind).iter().map(|e| e.key))
            .collect();
        // Literals that may lie outside the generated scope (format-excluded branches).
        keys.extend(["", "abilityshield", "perishbody", "persistent", "notakey"]);
        for key in keys {
            let set = KeyIds::of(key);
            assert!(!set.contains(EffectId::NONE));
            for id in 0..EFFECT_COUNT as u16 {
                let id = EffectId(id);
                let by_string = id != EffectId::NONE && effect(id).key == key;
                assert_eq!(set.contains(id), by_string, "{key} {id:?}");
            }
            assert_eq!(set.ids().len(), set.ids().iter().filter(|&&id| id != EffectId::NONE).count());
        }
    }
}
