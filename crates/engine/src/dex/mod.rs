//! Generated immutable Dex. Numeric ids and typed hot tables; integer-keyed trees
//! retain all declarative fields and their JS insertion order for effect authors.
use crate::ids::{EffectId, EffectKind, TypeId};
#[rustfmt::skip]
mod data_generated;
#[rustfmt::skip]
mod events_generated;
#[rustfmt::skip]
mod hooks_generated;
#[rustfmt::skip]
mod ids_generated;
pub use data_generated::*;
pub use events_generated::*;
pub use hooks_generated::*;
pub use ids_generated::*;
mod hook_index;
pub use hook_index::{
    callback_relations, effect_callback_relations, effect_has_callback, event_hook, has_callback,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DataValue {
    Null,
    Bool(bool),
    Number(f64),
    Text(&'static str),
    Array(&'static [DataValue]),
    Object(&'static [Property]),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Property {
    pub key: FieldId,
    pub value: DataValue,
}
impl DataValue {
    pub fn get(self, key: FieldId) -> Option<Self> {
        match self {
            Self::Object(props) => props.iter().find(|p| p.key == key).map(|p| p.value),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct EffectData {
    pub id: EffectId,
    pub key: &'static str,
    pub name: &'static str,
    pub data: DataValue,
}
#[derive(Clone, Copy, Debug)]
pub struct SpeciesConditionView {
    pub species: EffectId,
    pub name: &'static str,
    pub effect_type: EffectType,
    pub hooks: &'static [HookId],
}
pub fn canonical_effect(id: EffectId) -> EffectId {
    CONDITION_RULE_ALIASES
        .iter()
        .find(|(from, _)| *from == id)
        .map_or(id, |(_, to)| *to)
}
#[derive(Clone, Copy, Debug)]
pub struct AbilitySlot {
    pub id: EffectId,
    pub key: &'static str,
    pub name: &'static str,
}
#[derive(Clone, Copy, Debug)]
pub struct SpeciesData {
    pub id: EffectId,
    pub name: &'static str,
    pub base_species_name: &'static str,
    pub base_species: EffectId,
    pub types: [TypeId; 2],
    pub base_stats: [u16; 6],
    pub abilities: [AbilitySlot; 4],
    pub gender: &'static str,
    pub male_ratio: f64,
    pub weighthg: u16,
    pub max_hp: u16,
    pub tags: u32,
    pub battle_only: &'static [EffectId],
    pub changes_from: EffectId,
    pub required_items: &'static [EffectId],
    pub required_ability: EffectId,
    pub required_move: EffectId,
    pub required_tera_type: TypeId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accuracy {
    Always,
    Percent(u16),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Category {
    Physical,
    Special,
    Status,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum MoveTarget {
    #[default]
    Normal,
    SelfTarget,
    Any,
    AllAdjacentFoes,
    AllySide,
    AllAdjacent,
    AdjacentAlly,
    All,
    Allies,
    FoeSide,
    RandomNormal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StatId {
    Hp,
    Atk,
    Def,
    SpA,
    SpD,
    Spe,
    Accuracy,
    Evasion,
}
#[derive(Clone, Copy, Debug)]
pub struct BoostChange {
    pub stat: StatId,
    pub delta: i8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfSwitch {
    None,
    Switch,
    CopyVolatile,
    ShedTail,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfDestruct {
    None,
    Always,
    IfHit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageSpec {
    None,
    Level,
    Fixed(u16),
}
#[derive(Clone, Copy, Debug)]
pub struct MoveEffects {
    pub boosts: &'static [BoostChange],
    pub status: EffectId,
    pub volatile_status: EffectId,
    pub side_condition: EffectId,
    pub slot_condition: EffectId,
    pub weather: EffectId,
    pub terrain: EffectId,
    pub pseudo_weather: EffectId,
    pub heal: Option<[u16; 2]>,
    pub force_switch: bool,
    pub self_switch: SelfSwitch,
    pub self_effect: Option<&'static MoveEffects>,
    pub hooks: &'static [HookId],
}
#[derive(Clone, Copy, Debug)]
pub struct SecondaryData {
    pub chance: Option<u16>,
    pub effects: MoveEffects,
}
#[derive(Clone, Copy, Debug)]
pub struct MoveData {
    pub id: EffectId,
    pub name: &'static str,
    pub base_power: u16,
    pub accuracy: Accuracy,
    pub pp: u8,
    pub priority: i8,
    pub category: Category,
    pub move_type: TypeId,
    pub target: MoveTarget,
    pub flags: u64,
    pub crit_ratio: u8,
    pub no_pp_boosts: bool,
    pub ignore_immunity: bool,
    pub traits: u32,
    pub multihit: [u8; 2],
    pub recoil: Option<[u16; 2]>,
    pub drain: Option<[u16; 2]>,
    pub damage: DamageSpec,
    pub self_destruct: SelfDestruct,
    pub offensive_stat: Option<StatId>,
    pub defensive_stat: Option<StatId>,
    pub offensive_target: bool,
    pub effects: MoveEffects,
    pub self_boost: Option<&'static MoveEffects>,
    pub secondaries: &'static [SecondaryData],
}
#[derive(Clone, Copy, Debug)]
pub struct AbilityData {
    pub id: EffectId,
    pub flags: u64,
    pub suppress_weather: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct ItemData {
    pub id: EffectId,
    pub berry: bool,
    pub choice: bool,
    pub gem: bool,
    pub ignore_klutz: bool,
    pub fling_power: u8,
    pub natural_gift_power: u8,
    pub natural_gift_type: TypeId,
    pub on_plate: TypeId,
    pub forced_forme: EffectId,
    pub item_users: &'static [EffectId],
}
#[derive(Clone, Copy, Debug)]
pub struct NatureData {
    pub key: &'static str,
    pub plus: i8,
    pub minus: i8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EffectType {
    Pokemon,
    Move,
    Ability,
    Item,
    Condition,
    Status,
    Weather,
    Rule,
    ValidatorRule,
    Format,
    Ruleset,
    Terrain,
    /// Synthetic source object with no effectType property.
    Undefined,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HookRel {
    On,
    Source,
    Ally,
    Foe,
    Any,
    Direct,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HookValue {
    Absent,
    Function,
    Constant(DataValue),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct HookId(pub u16);
#[derive(Clone, Copy, Debug)]
pub struct Hook {
    pub effect: EffectId,
    pub key: &'static str,
    pub site: &'static str,
    pub event: EventId,
    pub rel: HookRel,
    pub order: f64,
    pub priority: f64,
    pub sub_order: f64,
    pub value: HookValue,
}
#[derive(Clone, Copy, Debug)]
pub struct EffectManifest {
    pub hooks_start: u16,
    pub hooks_len: u16,
    pub duration: Option<u16>,
    pub duration_callback: bool,
    pub effect_type: EffectType,
    pub events: [u64; EVENT_COUNT.div_ceil(64)],
}
impl EffectManifest {
    pub const EMPTY: Self = Self {
        hooks_start: 0,
        hooks_len: 0,
        duration: None,
        duration_callback: false,
        effect_type: EffectType::Condition,
        events: [0; EVENT_COUNT.div_ceil(64)],
    };
    pub fn hooks(&self) -> &'static [Hook] {
        &HOOKS[self.hooks_start as usize..(self.hooks_start + self.hooks_len) as usize]
    }
    pub fn has_event(&self, event: EventId) -> bool {
        self.events[event as usize / 64] & (1 << (event as usize % 64)) != 0
    }
}
pub fn species(id: EffectId) -> &'static SpeciesData {
    &SPECIES[(id.0 - SPECIES_START) as usize]
}
pub fn move_data(id: EffectId) -> &'static MoveData {
    &MOVES[(id.0 - MOVE_START) as usize]
}
pub fn table(kind: EffectKind) -> &'static [EffectData] {
    match kind {
        EffectKind::Species => SPECIES_DATA,
        EffectKind::Move => MOVES_DATA,
        EffectKind::Ability => ABILITIES_DATA,
        EffectKind::Item => ITEMS_DATA,
        EffectKind::Condition => CONDITIONS_DATA,
        EffectKind::Rule => RULES_DATA,
    }
}
pub fn effect(id: EffectId) -> &'static EffectData {
    let kind = id.kind().expect("invalid effect id");
    let entries = table(kind);
    &entries[(id.0 - entries[0].id.0) as usize]
}
/// Input boundary only. Battle dispatch never normalizes strings.
pub fn lookup(kind: EffectKind, text: &str) -> Option<EffectId> {
    let entries = table(kind);
    if text.len() <= 64 {
        let mut key = [0u8; 64];
        let mut len = 0;
        for byte in text.bytes().filter(u8::is_ascii_alphanumeric) {
            key[len] = byte.to_ascii_lowercase();
            len += 1;
        }
        entries
            .binary_search_by(|e| e.key.as_bytes().cmp(&key[..len]))
            .ok()
            .map(|i| entries[i].id)
    } else {
        // Preserve unbounded input handling without making common lookups allocate.
        let normalized: String = text
            .bytes()
            .filter(u8::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase() as char)
            .collect();
        entries
            .binary_search_by_key(&normalized.as_str(), |e| e.key)
            .ok()
            .map(|i| entries[i].id)
    }
}
pub fn type_id(text: &str) -> Option<TypeId> {
    // Dispatch on the first ASCII byte, then retain the original full-string
    // comparison. This keeps the generated TypeId ordering and rejects exactly
    // the same non-ASCII and malformed names as the former table scan.
    let first = text.as_bytes().first()?.to_ascii_lowercase();
    let id = match first {
        b'n' if text.eq_ignore_ascii_case("Normal") => 1,
        b'f' if text.eq_ignore_ascii_case("Fighting") => 2,
        b'f' if text.eq_ignore_ascii_case("Flying") => 3,
        b'p' if text.eq_ignore_ascii_case("Poison") => 4,
        b'g' if text.eq_ignore_ascii_case("Ground") => 5,
        b'r' if text.eq_ignore_ascii_case("Rock") => 6,
        b'b' if text.eq_ignore_ascii_case("Bug") => 7,
        b'g' if text.eq_ignore_ascii_case("Ghost") => 8,
        b's' if text.eq_ignore_ascii_case("Steel") => 9,
        b'f' if text.eq_ignore_ascii_case("Fire") => 10,
        b'w' if text.eq_ignore_ascii_case("Water") => 11,
        b'g' if text.eq_ignore_ascii_case("Grass") => 12,
        b'e' if text.eq_ignore_ascii_case("Electric") => 13,
        b'p' if text.eq_ignore_ascii_case("Psychic") => 14,
        b'i' if text.eq_ignore_ascii_case("Ice") => 15,
        b'd' if text.eq_ignore_ascii_case("Dragon") => 16,
        b'd' if text.eq_ignore_ascii_case("Dark") => 17,
        b'f' if text.eq_ignore_ascii_case("Fairy") => 18,
        b's' if text.eq_ignore_ascii_case("Stellar") => 19,
        b'?' if text == "???" => 20,
        _ => return None,
    };
    Some(TypeId(id))
}

/// `type_id(text).unwrap()` for an exact generated name, resolved at compile time.
const fn type_const(text: &str) -> TypeId {
    let mut i = 0;
    while i < TYPE_NAMES.len() {
        let (a, b) = (TYPE_NAMES[i].as_bytes(), text.as_bytes());
        if a.len() == b.len() {
            let mut j = 0;
            while j < a.len() && a[j] == b[j] {
                j += 1;
            }
            if j == a.len() {
                return TypeId(i as u8 + 1);
            }
        }
        i += 1;
    }
    panic!("unknown type name");
}
pub const TYPE_NORMAL: TypeId = type_const("Normal");
pub const TYPE_FIGHTING: TypeId = type_const("Fighting");
pub const TYPE_FLYING: TypeId = type_const("Flying");
pub const TYPE_POISON: TypeId = type_const("Poison");
pub const TYPE_GROUND: TypeId = type_const("Ground");
pub const TYPE_ROCK: TypeId = type_const("Rock");
pub const TYPE_BUG: TypeId = type_const("Bug");
pub const TYPE_GHOST: TypeId = type_const("Ghost");
pub const TYPE_STEEL: TypeId = type_const("Steel");
pub const TYPE_FIRE: TypeId = type_const("Fire");
pub const TYPE_WATER: TypeId = type_const("Water");
pub const TYPE_GRASS: TypeId = type_const("Grass");
pub const TYPE_ELECTRIC: TypeId = type_const("Electric");
pub const TYPE_PSYCHIC: TypeId = type_const("Psychic");
pub const TYPE_ICE: TypeId = type_const("Ice");
pub const TYPE_DRAGON: TypeId = type_const("Dragon");
pub const TYPE_DARK: TypeId = type_const("Dark");
pub const TYPE_FAIRY: TypeId = type_const("Fairy");
pub const TYPE_STELLAR: TypeId = type_const("Stellar");
pub const TYPE_UNKNOWN: TypeId = type_const("???");

#[cfg(test)]
mod construction_lookup_tests {
    use super::*;

    #[test]
    fn type_lookup_matches_generated_order_and_ascii_case_rules() {
        for (i, name) in TYPE_NAMES.iter().enumerate() {
            let expected = Some(TypeId(i as u8 + 1));
            assert_eq!(type_id(name), expected);
            assert_eq!(type_id(&name.to_ascii_lowercase()), expected);
            assert_eq!(type_id(&name.to_ascii_uppercase()), expected);
        }
        for invalid in ["", " normal", "Normal!", "grounded", "Ｆｉｒｅ", "???x"] {
            assert_eq!(type_id(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn effect_lookup_matches_allocating_normalization() {
        let kinds = [
            EffectKind::Species,
            EffectKind::Move,
            EffectKind::Ability,
            EffectKind::Item,
            EffectKind::Condition,
            EffectKind::Rule,
        ];
        for kind in kinds {
            for effect in table(kind) {
                let decorated = format!("{} !", effect.key.to_ascii_uppercase());
                let normalized: String = decorated
                    .bytes()
                    .filter(u8::is_ascii_alphanumeric)
                    .map(|byte| byte.to_ascii_lowercase() as char)
                    .collect();
                let entries = table(kind);
                let old = entries
                    .binary_search_by_key(&normalized.as_str(), |entry| entry.key)
                    .ok()
                    .map(|i| entries[i].id);
                assert_eq!(lookup(kind, &decorated), old, "{kind:?} {decorated}");
            }
            let long = format!("{}Normal", "!".repeat(65));
            let normalized = "normal";
            let entries = table(kind);
            let old = entries
                .binary_search_by_key(&normalized, |entry| entry.key)
                .ok()
                .map(|i| entries[i].id);
            assert_eq!(lookup(kind, &long), old, "{kind:?} long input");
        }
    }
}
