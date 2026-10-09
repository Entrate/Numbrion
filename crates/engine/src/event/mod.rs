//! Event system owner: lead engine author (stage 2B).
//! Pinned battle.ts:408-1251. Metadata comes exclusively from the generated dex.
#![allow(unused_variables)]
pub mod collect;
pub mod dispatch;
pub mod modifiers;
pub mod order;
pub mod scratch;
pub mod suppression;
pub mod types;
pub use crate::state::scratch::{
    CallArgs, EffectRef, EventArg, EventFrame, HookCtx, Relay, SyntheticEffect,
};
pub use order::{SortOrder, SpeedSortable, compare_priority, speed_sort};
pub use scratch::Scratch;
pub use types::*;

mod lookup;
