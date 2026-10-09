//! Stable battle identities and a single contiguous generated effect-id space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct EffectId(pub u16);
impl EffectId {
    pub const NONE: Self = Self(0);
    pub const fn kind(self) -> Option<EffectKind> {
        use crate::dex::*;
        Some(if self.0 >= SPECIES_START && self.0 < SPECIES_END {
            EffectKind::Species
        } else if self.0 >= MOVE_START && self.0 < MOVE_END {
            EffectKind::Move
        } else if self.0 >= ABILITY_START && self.0 < ABILITY_END {
            EffectKind::Ability
        } else if self.0 >= ITEM_START && self.0 < ITEM_END {
            EffectKind::Item
        } else if self.0 >= CONDITION_START && self.0 < CONDITION_END {
            EffectKind::Condition
        } else if self.0 >= RULE_START && self.0 < RULE_END {
            EffectKind::Rule
        } else {
            return None;
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Species,
    Move,
    Ability,
    Item,
    Condition,
    Rule,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct MonId(pub u8);
impl MonId {
    pub const NONE: Self = Self(u8::MAX);
    pub const fn side(self) -> SideId {
        SideId(self.0 / 6)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SideId(pub u8);
/// Position can be 0..5: attackedBy.sourceSlot can name a now-benched source.
/// Encoding is side * 6 + party position; field-position order is a separate permutation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SlotId(pub u8);
impl SlotId {
    pub const NONE: Self = Self(u8::MAX);
    pub const fn new(side: SideId, position: u8) -> Self {
        Self(side.0 * 6 + position)
    }
    pub const fn side(self) -> SideId {
        SideId(self.0 / 6)
    }
    pub const fn position(self) -> u8 {
        self.0 % 6
    }
    pub const fn field_position(self) -> u8 {
        self.side().0 + 2 * self.position()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct TypeId(pub u8);
impl TypeId {
    pub const NONE: Self = Self(0);
}
/// Tagged holder: 0..11 Pokemon, 12..13 Side, 14 Field, 15 Battle, 255 absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Holder(pub u8);
impl Holder {
    pub const NONE: Self = Self(255);
    pub const FIELD: Self = Self(14);
    pub const BATTLE: Self = Self(15);
    pub const fn mon(mon: MonId) -> Self {
        Self(mon.0)
    }
    pub const fn side(side: SideId) -> Self {
        Self(12 + side.0)
    }
}
pub use crate::dex::EventId;
