//! Cross-owner action values: battle-actions.ts:210,546,1014,1137,1572.
//! Values contain no allocations. Constructing them makes no PRNG draws.
use crate::{
    ids::*,
    state::scratch::{EffectRef, EventArg, MoveEffectsScratch, Relay},
};

/// Scratch index, never retained in a Copy decision snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveHandle(pub u8);
impl MoveHandle {
    pub const NONE: Self = Self(255);
}
/// Recharge is a synthetic nonexistent move, distinct from a scoped dex ID.
#[derive(Clone, Copy, Debug)]
pub enum MoveInput {
    Dex(EffectId),
    Active(MoveHandle),
    Recharge,
}
/// getDamage also accepts a synthetic numeric base power (battle-actions.ts:1572).
#[derive(Clone, Copy, Debug)]
pub enum DamageInput {
    Move(MoveInput),
    BasePower(f64),
}
/// Preserve undefined versus explicit null sources for defaulting in mutators.
#[derive(Clone, Copy, Debug)]
pub struct Attribution {
    pub source: EventArg,
    pub effect: EffectRef,
}
impl Attribution {
    pub const DEFAULT: Self = Self {
        source: EventArg::Undefined,
        effect: EffectRef::None,
    };
    pub const NONE: Self = Self {
        source: EventArg::Null,
        effect: EffectRef::None,
    };
    pub const fn from_move(source: MonId, effect: EffectRef) -> Self {
        Self {
            source: EventArg::Holder(Holder::mon(source)),
            effect,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RunMoveOptions {
    pub external: bool,
    pub original_target: Option<MonId>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct UseMoveOptions {
    pub external: bool,
    pub z_move: bool,
}
/// Null (Substitute) continues self effects; False terminates that target's hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    Pokemon(MonId),
    False,
    Null,
}
#[derive(Clone, Copy, Debug)]
pub struct Targets {
    pub entries: [HitTarget; 4],
    pub len: u8,
}
impl Default for Targets {
    fn default() -> Self {
        Self {
            entries: [HitTarget::False; 4],
            len: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveTargets {
    pub hit: Targets,
    pub pressure: Targets,
}
#[derive(Clone, Copy, Debug)]
pub struct TargetResults {
    pub values: [Relay; 4],
    pub len: u8,
}
impl Default for TargetResults {
    fn default() -> Self {
        Self {
            values: [Relay::Undefined; 4],
            len: 0,
        }
    }
}
/// Nested self/secondary object (not a new ActiveMove and not a new event frame).
#[derive(Clone, Copy, Debug)]
pub enum HitEffect {
    Primary,
    SelfEffect,
    SelfBoost,
    Secondary(u8),
    SecondarySelf(u8),
    Scratch(MoveEffectsScratch),
}
#[derive(Clone, Copy, Debug, Default)]
pub struct HitOptions {
    pub secondary: bool,
    pub self_hit: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DamageOptions {
    pub suppress_messages: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Stat {
    Atk,
    Def,
    SpA,
    SpD,
    Spe,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct StatOptions {
    pub unboosted: bool,
    pub unmodified: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Types {
    pub values: [TypeId; 3],
    pub len: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealEffect {
    Context,
    Effect(EffectRef),
    Drain,
}
#[derive(Clone, Copy, Debug)]
pub enum ImmunitySource {
    Type(TypeId),
    Move(MoveHandle),
}
#[derive(Clone, Copy, Debug)]
pub enum ImmunityMessage {
    Silent,
    Standard,
    Text(&'static str),
}
#[derive(Clone, Copy, Debug)]
pub struct FormeOptions {
    pub permanent: bool,
    pub message: Option<&'static str>,
}
