//! Event and move values. Scratch is excluded from BattleState snapshots; queued
//! moves and requests in BattleState retain everything needed at a decision boundary.
use crate::{
    dex::{Category, DamageSpec, ImmunityId, MoveEffects, MoveTarget, SelfDestruct, StatId},
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
#[repr(u8)]
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
    /// Live mutable move object; never persisted in a decision snapshot.
    ActiveMove(u8),
    SpeciesCondition(EffectId),
    MoveCondition(EffectId),
    AbilityCondition(EffectId),
    ItemCondition(EffectId),
    Synthetic(SyntheticEffect),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventArg {
    /// Typed relay as an explicit callback argument, including arrays/objects in scratch.
    Relay(Relay),
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
    pub chance: Option<u16>,
    pub heal: Option<[u16; 2]>,
    pub force_switch: bool,
    pub self_switch: crate::dex::SelfSwitch,
    /// Bit i disables base.hooks[i], e.g. Curse deleting its primary onHit.
    pub suppressed_hooks: u16,
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
    pub traits: u32,
    pub runtime_flags: u32,
    pub total_damage: u32,
    pub hit_targets: [MonId; 4],
    pub hit_target_len: u8,
    pub id: EffectId,
    pub source_effect: EffectRef,
    pub type_changer_boosted: EffectId,
    /// Atk/Def/SpA/SpD Ruin holder identities; MonId::NONE means absent.
    pub ruined_stats: [MonId; 4],
    pub base_power: f64,
    pub accuracy: MoveAccuracy,
    pub hit_data: [HitData; 12],
    /// Primary boosts live in effects.boosts; this is the separate selfBoost object.
    pub self_boosts: OrderedBoosts,
    pub effects: MoveEffectsScratch,
    pub self_effect: Option<MoveEffectsScratch>,
    pub secondaries: [Option<SecondaryScratch>; 4],
    pub secondary_count: u8,
    pub secondaries_present: bool,
    pub recoil: Option<[u16; 2]>,
    pub drain: Option<[u16; 2]>,
    pub damage: DamageSpec,
    pub self_destruct: SelfDestruct,
    pub offensive_stat: Option<StatId>,
    pub defensive_stat: Option<StatId>,
    pub offensive_target: bool,
    pub ignore_immunity_types: u32,
    pub move_type: TypeId,
    pub category: Category,
    pub target: MoveTarget,
    pub priority: i8,
    pub crit_ratio: u8,
    pub hit: u8,
    pub multihit: [u8; 2],
}
/// Mutable move numbers preserve fractional edits before TS rounds/truncates them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveAccuracy {
    Always,
    Percent(f64),
}
/// Mutable JS properties have their own overlay bits, separate from dex flags.
/// Ports battle-actions.ts:390-442,850-1013 and scoped ability/item ModifyMove.
pub mod move_runtime {
    pub const EXTERNAL: u32 = 1 << 0;
    pub const HAS_BOUNCED: u32 = 1 << 1;
    pub const HAS_SHEER_FORCE: u32 = 1 << 2;
    pub const IGNORE_ABILITY: u32 = 1 << 3;
    pub const IGNORE_EVASION: u32 = 1 << 4;
    pub const INFILTRATES: u32 = 1 << 5;
    pub const PRANKSTER_BOOSTED: u32 = 1 << 6;
    pub const TRACKS_TARGET: u32 = 1 << 7;
    pub const SMART_TARGET: u32 = 1 << 8;
    pub const SMART_TARGET_PRESENT: u32 = 1 << 9;
    pub const SPREAD_HIT: u32 = 1 << 10;
    pub const LAST_HIT: u32 = 1 << 11;
    pub const SELF_DROPPED: u32 = 1 << 12;
    pub const MULTIACCURACY: u32 = 1 << 13;
    pub const STELLAR_BOOSTED: u32 = 1 << 14;
    pub const IGNORE_IMMUNITY: u32 = 1 << 15;
    pub const IGNORE_IMMUNITY_PRESENT: u32 = 1 << 16;
    pub const MULTIHIT_PRESENT: u32 = 1 << 17;
    pub const MULTIHIT_RANGE: u32 = 1 << 18;
    pub const WILL_CRIT: u32 = 1 << 19;
    pub const WILL_CRIT_PRESENT: u32 = 1 << 20;
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
