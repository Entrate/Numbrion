//! numbrion differential-test harness. See docs/design/DIFFTEST.md.
//!
//! The harness replays Showdown oracle fixtures (docs/design/FIXTURES.md) through anything that
//! implements [`sim::Sim`] and reports the first divergence per battle.

#![allow(clippy::needless_range_loop)]

pub mod diff;
pub mod driver;
#[cfg(feature = "engine")]
pub mod engine_sim;
pub mod fixture;
pub mod mock;
pub mod replay;
pub mod report;
pub mod runner;
pub mod seed;
pub mod selftest;
pub mod sim;
pub mod stats;
