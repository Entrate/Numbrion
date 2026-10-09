use crate::{dex::MoveTarget, ids::*};
/// Resolved queue moves persist across mid-turn requests. Their pre-execution
/// mutations are stored here; a fresh ActiveMove scratch frame resumes this overlay.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct QueuedMove {
    pub id: EffectId,
    pub flags: u16,
    pub priority: i8,
    pub kind: ChosenMoveKind,
}
/// Recharge is not a real Dex move (pokemon.ts:949,964; side.ts:552).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ChosenMoveKind {
    #[default]
    Dex,
    Recharge,
}
/// Queue-time ModifyPriority overlays (battle-queue.ts:249; battle.ts:2649-2653).
/// PRNG: none to encode; the modifying event owns any draws.
pub mod queued_move_flags {
    pub const PRANKSTER_BOOSTED: u16 = 1 << 0;
    pub const IGNORE_ABILITY: u16 = 1 << 1;
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ActionKind {
    #[default]
    None,
    Start,
    BeforeTurn,
    BeforeTurnMove,
    PriorityChargeMove,
    Move,
    Switch,
    InstaSwitch,
    RevivalBlessing,
    RunSwitch,
    Terastallize,
    Residual,
    Pass,
    Event,
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Action {
    pub priority: f64,
    pub fractional_priority: f64,
    pub move_data: QueuedMove,
    pub source_effect: EffectId,
    pub order: u16,
    pub speed: u16,
    pub kind: ActionKind,
    pub pokemon: MonId,
    pub target: MonId,
    pub original_target: MonId,
    pub target_loc: i8,
    pub tera: TypeId,
    pub event: Option<EventId>,
    pub target_loc_present: bool,
}
impl Default for Action {
    fn default() -> Self {
        Self {
            priority: 0.0,
            fractional_priority: 0.0,
            move_data: QueuedMove::default(),
            source_effect: EffectId::NONE,
            order: 0,
            speed: 0,
            kind: ActionKind::None,
            pokemon: MonId::NONE,
            target: MonId::NONE,
            original_target: MonId::NONE,
            target_loc: 0,
            tera: TypeId::NONE,
            event: None,
            target_loc_present: false,
        }
    }
}
pub const QUEUE_CAPACITY: usize = 64;
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ActionQueue {
    pub entries: [Action; QUEUE_CAPACITY],
    pub len: u16,
}
impl Default for ActionQueue {
    fn default() -> Self {
        Self {
            entries: [Action::default(); QUEUE_CAPACITY],
            len: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ChoiceKind {
    #[default]
    Pass,
    Move,
    Switch,
    Revival,
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SlotChoice {
    pub kind: ChoiceKind,
    pub move_slot: u8,
    pub switch_to: MonId,
    pub target_loc: i8,
    pub tera: bool,
    /// Frozen validated move; move_slot=255 when there is no original PP slot.
    pub move_id: EffectId,
    pub move_kind: ChosenMoveKind,
}
impl Default for SlotChoice {
    fn default() -> Self {
        Self {
            kind: ChoiceKind::Pass,
            move_slot: 255,
            switch_to: MonId::NONE,
            target_loc: 0,
            tera: false,
            move_id: EffectId::NONE,
            move_kind: ChosenMoveKind::Dex,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct SideChoice {
    pub slots: [SlotChoice; 2],
    pub len: u8,
    pub switch_ins: u8,
    pub forced_switches_left: u8,
    pub forced_passes_left: u8,
    pub cant_undo: bool,
    pub tera: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum RequestKind {
    #[default]
    None,
    Wait,
    Move,
    Switch,
}
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct RequestMove {
    pub id: EffectId,
    pub disabled_source: EffectId,
    pub pp: u8,
    pub max_pp: u8,
    pub target: MoveTarget,
    pub disabled: u8,
    pub presence: u8,
}
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct ActiveRequest {
    pub moves: [RequestMove; 4],
    pub move_count: u8,
    pub flags: u16,
    pub can_tera: TypeId,
}
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct SideRequest {
    pub active: [ActiveRequest; 2],
    pub kind: RequestKind,
    pub force_switch_mask: u8,
    pub no_cancel: bool,
    pub update: bool,
}
