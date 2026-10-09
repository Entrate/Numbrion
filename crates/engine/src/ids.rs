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

/// Copy attribution at a queued faint (pokemon.ts:1581; battle.ts:2539).
/// Low 13 bits are a dex ID (or synthetic ordinal); high 3 bits select its view.
/// PRNG: none. Live ActiveMove must first be projected through Battle.freeze_effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct EffectToken(pub u16);
impl EffectToken {
    pub const NONE: Self = Self(0);
    pub const fn from_ref(effect: crate::state::scratch::EffectRef) -> Self {
        use crate::state::scratch::EffectRef::*;
        let (view, id) = match effect {
            None => (0, 0),
            Dex(id) => (0, id.0),
            SpeciesCondition(id) => (1, id.0),
            MoveCondition(id) => (2, id.0),
            AbilityCondition(id) => (3, id.0),
            ItemCondition(id) => (4, id.0),
            Synthetic(s) => (5, s as u16),
            Synchronize(status) => (6, status.0),
            ActiveMove(_) => panic!("cannot store a live move in a snapshot"),
        };
        assert!(id < 8192, "effect token ID overflow");
        Self((view << 13) | id)
    }
    pub const fn resolve(self) -> crate::state::scratch::EffectRef {
        use crate::state::scratch::{EffectRef::*, SyntheticEffect};
        let id = EffectId(self.0 & 8191);
        match self.0 >> 13 {
            0 => {
                if id.0 == 0 {
                    None
                } else {
                    Dex(id)
                }
            }
            1 => SpeciesCondition(id),
            2 => MoveCondition(id),
            3 => AbilityCondition(id),
            4 => ItemCondition(id),
            5 => Synthetic(match id.0 {
                0 => SyntheticEffect::Format,
                1 => SyntheticEffect::Confused,
                2 => SyntheticEffect::StruggleRecoil,
                3 => SyntheticEffect::Recharge,
                4 => SyntheticEffect::Fainted,
                5 => SyntheticEffect::MindBlownRecoil,
                _ => panic!("invalid synthetic effect token"),
            }),
            6 => Synchronize(id),
            _ => panic!("invalid effect token view"),
        }
    }
}
const _: () = assert!(core::mem::size_of::<EffectToken>() == 2);
