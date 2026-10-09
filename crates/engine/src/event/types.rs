//! Event invocation/captured-listener contracts; battle.ts:575,762,954,1039.
//! Constructing these values draws nothing. Dispatch and ordering own all draws.
use crate::{
    dex::{HookId, HookRel},
    ids::*,
    state::{
        CellRef,
        scratch::{EffectRef, EventArg, Relay},
    },
};
#[derive(Clone, Copy, Debug, Default)]
pub struct RunEventOptions {
    pub on_effect: bool,
    pub fast_exit: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct EventTargets {
    pub mons: [MonId; 4],
    pub len: u8,
}
#[derive(Clone, Copy, Debug)]
pub enum EventTarget {
    Single(EventArg),
    Spread(EventTargets),
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Priority {
    pub order: f64,
    pub priority: f64,
    pub speed: f64,
    pub sub_order: f64,
    pub effect_order: u32,
    pub index: u8,
    pub redirect_order: Option<u32>,
}
#[derive(Clone, Copy, Debug)]
pub struct Listener {
    pub effect: EffectRef,
    /// Requested callback key; SwitchIn fallback dispatch still uses SwitchIn ordering.
    pub selector: HookSelector,
    pub hook: Option<HookId>,
    pub state: Option<CellRef>,
    pub holder: Holder,
    pub priority: Priority,
    pub target_index: u8,
    pub end: EndHandler,
}
#[derive(Clone, Copy, Debug)]
pub enum EndHandler {
    None,
    Status(MonId),
    Ability(MonId),
    Item(MonId),
    Volatile(MonId, EffectId),
    Side(SideId, EffectId),
    Slot(SlotId, EffectId),
    Weather,
    Terrain,
    PseudoWeather(EffectId),
}
#[derive(Clone, Copy, Debug)]
pub enum CollectMode {
    Callback,
    Duration,
}
/// One fixed buffer per nested event; preserves duplicates and pins captured cells.
#[derive(Debug)]
pub struct HandlerBuffer {
    pub entries: Box<[Option<Listener>]>,
    pub len: u16,
}
impl Default for HandlerBuffer {
    fn default() -> Self {
        Self {
            entries: vec![None; HANDLER_CAPACITY].into_boxed_slice(),
            len: 0,
        }
    }
}
pub const EVENT_DEPTH: usize = 8;
pub const MOVE_DEPTH: usize = 8;
pub const CALL_DEPTH: usize = 16;
pub const HANDLER_CAPACITY: usize = 2304;
/// Callback selector is metadata, not a string assembled at runtime.
#[derive(Clone, Copy, Debug)]
pub struct HookSelector {
    pub event: EventId,
    pub rel: HookRel,
    pub mode: CollectMode,
}
/// Single relay is distinct from per-target relay even for a singleton target array.
#[derive(Clone, Copy, Debug)]
pub enum EventResult {
    Single(Relay),
    Spread(crate::actions::TargetResults),
}
