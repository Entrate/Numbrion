//! OWNER M: move execution, targeting, hit stages, and mutable move frames.
//! Ports sim/battle-actions.ts:210-1387 and its targeting/locking helpers.
//! Only this directory belongs to M; request cross-owner signature changes from
//! their owner. All behavior bodies remain unimplemented during stage 2A.
pub mod active;
pub mod effects;
pub mod execution;
pub mod hit;
pub mod targeting;
