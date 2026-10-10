//! OWNER M: move execution, targeting, hit stages, and mutable move frames.
//! Ports sim/battle-actions.ts:210-1387 and its targeting/locking helpers.
//! Only this directory belongs to M; request cross-owner signature changes from
//! their owner.
pub mod active;
pub mod effects;
pub mod execution;
pub mod hit;
mod support;
pub mod targeting;
/// Empty immutable effects object: the `base` for a runtime-created `self`/secondary
/// object (e.g. Curse's `move.self = {boosts}`), since scratch effects borrow immutable data.
pub use support::EMPTY_EFFECTS;
