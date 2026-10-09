//! Event and move values. Scratch is excluded from BattleState snapshots; queued
//! moves and requests in BattleState retain everything needed at a decision boundary.
use crate::{
    dex::{Accuracy, Category, DamageSpec, ImmunityId, MoveEffects, MoveTarget},
    ids::*,
};
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Relay {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    NotFail,
    Pokemon(MonId),
    Effect(EffectId),
    Type(TypeId),
    Move(EffectId),
    Types(u8),
    Boosts(u8),
    ActiveMove(u8),
    Secondaries(u8),
    Species(EffectId),
    PseudoMove(SyntheticEffect),
    StatusImmunity(ImmunityId),
}
impl Relay {
    pub const FAIL: Self = Self::Bool(false);
    pub const SILENT_FAIL: Self = Self::Null;
    /// The sentinel is JS numeric zero, never a separate truthy variant.
    pub const HIT_SUBSTITUTE: Self = Self::Number(0.0);
    pub fn truthy(self) -> bool {
        match self {
            Self::Undefined | Self::Null | Self::Bool(false) | Self::NotFail => false,
            Self::Number(n) => n != 0.0 && !n.is_nan(),
            _ => true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntheticEffect {
    Format,
    Confused,
    StruggleRecoil,
    Recharge,
    Fainted,
    MindBlownRecoil,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectRef {
    None,
    Dex(EffectId),
    SpeciesCondition(EffectId),
    MoveCondition(EffectId),
    AbilityCondition(EffectId),
    ItemCondition(EffectId),
    Synthetic(SyntheticEffect),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventArg {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Holder(Holder),
    Effect(EffectRef),
    Type(TypeId),
    StatusImmunity(ImmunityId),
    Move(u8),
    StaticText(&'static str),
    Slot(SlotId),
}
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct OrderedBoosts {
    pub values: [i8; 7],
    pub order: [u8; 7],
    pub len: u8,
    pub present: u8,
}
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct HitData {
    pub crit: bool,
    pub type_mod: i8,
    pub bypass_protect: bool,
    pub present: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct MoveEffectsScratch {
    pub base: &'static MoveEffects,
    pub boosts: OrderedBoosts,
    pub status: EffectId,
    pub volatile_status: EffectId,
    pub side_condition: EffectId,
    pub slot_condition: EffectId,
    pub weather: EffectId,
    pub terrain: EffectId,
    pub pseudo_weather: EffectId,
}
#[derive(Clone, Copy, Debug)]
pub struct SecondaryScratch {
    pub effects: MoveEffectsScratch,
    pub self_effect: Option<MoveEffectsScratch>,
    pub chance: u16,
    pub chance_present: bool,
}
/// Generic move mutation overlay. immutable declarative data remains in Dex.
/// Self/secondary edits use ordered typed scratch payloads in stage 3.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ActiveMove {
    pub flags: u64,
    pub runtime_flags: u32,
    pub total_damage: u32,
    pub hit_targets: [MonId; 4],
    pub hit_target_len: u8,
    pub id: EffectId,
    pub source_effect: EffectRef,
    pub base_power: u16,
    pub accuracy: Accuracy,
    pub hit_data: [HitData; 12],
    pub boosts: OrderedBoosts,
    pub self_boosts: OrderedBoosts,
    pub effects: MoveEffectsScratch,
    pub self_effect: Option<MoveEffectsScratch>,
    pub secondaries: [Option<SecondaryScratch>; 4],
    pub secondary_count: u8,
    pub secondaries_present: bool,
    pub recoil: Option<[u16; 2]>,
    pub drain: Option<[u16; 2]>,
    pub damage: DamageSpec,
    pub ignore_immunity_types: u32,
    pub move_type: TypeId,
    pub category: Category,
    pub target: MoveTarget,
    pub priority: i8,
    pub crit_ratio: u8,
    pub hit: u8,
    pub multihit: [u8; 2],
}
#[derive(Clone, Copy, Debug)]
pub struct EventFrame {
    pub event: EventId,
    pub target: EventArg,
    pub source: EventArg,
    pub source_effect: EffectRef,
    pub relay: Relay,
    pub modifier: u32,
    pub modifier_present: bool,
    pub has_relay: bool,
    pub fast_exit: bool,
    pub args: [EventArg; 4],
    pub arg_count: u8,
}
#[derive(Clone, Copy, Debug)]
pub struct HookCtx {
    pub frame: u8,
    pub call_args: u8,
    pub active_move: u8,
    pub state: super::CellRef,
    pub holder: Holder,
    pub target: EventArg,
    pub source: EventArg,
    pub source_effect: EffectRef,
    pub relay: Relay,
    pub target_index: u8,
}
const _: () = assert!(core::mem::size_of::<Relay>() == 16);
#[derive(Clone, Copy, Debug)]
pub struct CallArgs {
    pub values: [EventArg; 4],
    pub len: u8,
}
