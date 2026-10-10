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

/// Every raw id whose generated key equals one of a few literals, across all
/// kinds and without alias canonicalization: `contains(id)` is exactly
/// `id != EffectId::NONE && keys.contains(&dex::effect(id).key)`. A key outside
/// the generated scope adds nothing, so it never matches (and never fabricates
/// an id). Build with `key_ids!` to force compile-time evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyIds {
    ids: [EffectId; KeyIds::CAPACITY],
    len: u8,
}
impl KeyIds {
    const CAPACITY: usize = 16;
    pub const fn of(key: &str) -> Self {
        Self::any(&[key])
    }
    pub const fn any(keys: &[&str]) -> Self {
        let mut set = Self {
            ids: [EffectId::NONE; Self::CAPACITY],
            len: 0,
        };
        let mut id = 1;
        while id < EFFECT_COUNT {
            let key = effect(EffectId(id as u16)).key;
            let mut k = 0;
            while k < keys.len() {
                if same_key(key, keys[k]) {
                    assert!((set.len as usize) < Self::CAPACITY, "key set capacity");
                    set.ids[set.len as usize] = EffectId(id as u16);
                    set.len += 1;
                    break;
                }
                k += 1;
            }
            id += 1;
        }
        set
    }
    #[inline(always)]
    pub fn contains(self, id: EffectId) -> bool {
        self.ids[..self.len as usize].contains(&id)
    }
}

/// `dex::lookup(kind, key)` for a literal that is already a normalized id
/// (lowercase ASCII alphanumerics, checked), usable in constants.
pub const fn key_id(kind: EffectKind, key: &str) -> Option<EffectId> {
    let bytes = key.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        assert!(
            bytes[i].is_ascii_lowercase() || bytes[i].is_ascii_digit(),
            "key_id needs a normalized key"
        );
        i += 1;
    }
    let entries = table(kind);
    let mut e = 0;
    while e < entries.len() {
        if same_key(entries[e].key, key) {
            return Some(entries[e].id);
        }
        e += 1;
    }
    None
}

/// `dex::KeyIds::any(&[keys...])`, evaluated at compile time.
macro_rules! key_ids {
    ($($key:expr),+ $(,)?) => {
        const { $crate::dex::KeyIds::any(&[$($key),+]) }
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

/// `IMMUNITY_NAMES` column of a literal immunity name; unknown names fail to compile.
pub const fn immunity_column(name: &str) -> usize {
    let mut i = 0;
    while i < IMMUNITY_NAMES.len() {
        if same_key(IMMUNITY_NAMES[i], name) {
            return i;
        }
        i += 1;
    }
    panic!("unknown immunity name")
}

const NO_IMMUNITY: u8 = u8::MAX;
static IMMUNITY_COLUMNS: [u8; EFFECT_COUNT] = {
    let mut table = [NO_IMMUNITY; EFFECT_COUNT];
    let mut id = 1;
    while id < EFFECT_COUNT {
        let key = effect(EffectId(id as u16)).key;
        let mut i = 0;
        while i < IMMUNITY_NAMES.len() {
            if same_key(IMMUNITY_NAMES[i], key) {
                table[id] = i as u8;
                break;
            }
            i += 1;
        }
        id += 1;
    }
    table
};

/// `IMMUNITY_NAMES.iter().position(|n| *n == dex::effect(id).key)` for a non-NONE id.
#[inline]
pub fn immunity_column_of(id: EffectId) -> Option<usize> {
    let i = IMMUNITY_COLUMNS[id.0 as usize];
    (i != NO_IMMUNITY).then_some(i as usize)
}

const fn is_key_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit()
}

/// Every generated effect key is lowercase alphanumeric, and every present species
/// ability slot's display name has another byte, so `slot.name == effect(id).key`
/// (Showdown's `abilityName === source.ability`) is false for every id.
pub const ABILITY_SLOT_NAMES_ARE_NOT_KEYS: bool = {
    let mut id = 1;
    while id < EFFECT_COUNT {
        let key = effect(EffectId(id as u16)).key.as_bytes();
        let mut i = 0;
        while i < key.len() {
            assert!(is_key_byte(key[i]), "effect key is not normalized");
            i += 1;
        }
        id += 1;
    }
    let mut s = 0;
    while s < SPECIES.len() {
        let mut a = 0;
        while a < SPECIES[s].abilities.len() {
            let slot = &SPECIES[s].abilities[a];
            if !slot.key.is_empty() {
                let name = slot.name.as_bytes();
                let mut plain = true;
                let mut i = 0;
                while i < name.len() {
                    plain &= is_key_byte(name[i]);
                    i += 1;
                }
                assert!(!plain, "an ability slot name could equal an effect key");
            }
            a += 1;
        }
        s += 1;
    }
    true
};
const _: () = assert!(ABILITY_SLOT_NAMES_ARE_NOT_KEYS);

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
    fn immunity_columns_match_name_search_for_every_id() {
        for id in 1..EFFECT_COUNT as u16 {
            let key = effect(EffectId(id)).key;
            let by_name = IMMUNITY_NAMES.iter().position(|name| *name == key);
            assert_eq!(immunity_column_of(EffectId(id)), by_name, "{key}");
        }
        for (i, name) in IMMUNITY_NAMES.iter().enumerate() {
            assert_eq!(immunity_column(name), i);
        }
    }

    /// Battle code compares species ids where Showdown compares names; each pair
    /// below is checked over the whole generated species table.
    #[test]
    fn species_name_checks_match_their_id_forms() {
        let eternamax = key_id(EffectKind::Species, "eternatuseternamax");
        for (i, s) in SPECIES.iter().enumerate() {
            assert_eq!(s.id, EffectId(SPECIES_START + i as u16));
            assert_eq!(species(s.id).id, s.id);
            assert_eq!(s.name == "Pecharunt", s.id == SPECIES_PECHARUNT, "{}", s.name);
            assert_eq!(s.name == "Eternatus-Eternamax", Some(s.id) == eternamax, "{}", s.name);
            let silvally = key_id(EffectKind::Species, "silvally");
            assert_eq!(s.base_species_name == "Silvally", Some(s.base_species) == silvally);
            for (name, id) in [
                ("Arceus", SPECIES_ARCEUS),
                ("Pikachu", SPECIES_PIKACHU),
                ("Ogerpon", SPECIES_OGERPON),
                ("Morpeko", SPECIES_MORPEKO),
                ("Terapagos", SPECIES_TERAPAGOS),
            ] {
                assert_eq!(s.base_species_name == name, s.base_species == id, "{}", s.name);
            }
            for (prefix, ids) in [
                (
                    "Ogerpon-Cornerstone",
                    [SPECIES_OGERPONCORNERSTONE, SPECIES_OGERPONCORNERSTONETERA],
                ),
                (
                    "Ogerpon-Hearthflame",
                    [SPECIES_OGERPONHEARTHFLAME, SPECIES_OGERPONHEARTHFLAMETERA],
                ),
                (
                    "Ogerpon-Wellspring",
                    [SPECIES_OGERPONWELLSPRING, SPECIES_OGERPONWELLSPRINGTERA],
                ),
            ] {
                assert_eq!(s.name.starts_with(prefix), ids.contains(&s.id), "{}", s.name);
            }
        }
    }

    #[test]
    fn no_ability_slot_name_equals_an_effect_key() {
        for species in SPECIES {
            for slot in species.abilities.iter().filter(|s| !s.key.is_empty()) {
                for id in 1..EFFECT_COUNT as u16 {
                    assert_ne!(slot.name, effect(EffectId(id)).key, "{}", species.name);
                }
            }
        }
    }

    #[test]
    fn key_id_matches_lookup_for_every_key_of_every_kind() {
        for kind in KINDS {
            for other in KINDS {
                for e in table(other) {
                    assert_eq!(key_id(kind, e.key), lookup(kind, e.key), "{kind:?} {}", e.key);
                }
            }
            for key in ["", "abilityshield", "perishbody", "notakey"] {
                assert_eq!(key_id(kind, key), lookup(kind, key), "{kind:?} {key}");
            }
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
        let by_string = |keys: &[&str], id: EffectId| {
            id != EffectId::NONE && keys.contains(&effect(id).key)
        };
        for (i, &key) in keys.iter().enumerate() {
            let set = KeyIds::of(key);
            // Unions: each key with its neighbour, as `key_ids!("a", "b")` builds them.
            let pair = [key, keys[(i + 1) % keys.len()]];
            let union = KeyIds::any(&pair);
            for id in 0..EFFECT_COUNT as u16 {
                let id = EffectId(id);
                assert_eq!(set.contains(id), by_string(&[key], id), "{key} {id:?}");
                assert_eq!(union.contains(id), by_string(&pair, id), "{pair:?} {id:?}");
            }
        }
        // The widest union used by battle code (effective_weather's sun/rain check).
        let weather = ["sunnyday", "raindance", "desolateland", "primordialsea"];
        let set = KeyIds::any(&weather);
        for id in 0..EFFECT_COUNT as u16 {
            assert_eq!(set.contains(EffectId(id)), by_string(&weather, EffectId(id)));
        }
    }
}
