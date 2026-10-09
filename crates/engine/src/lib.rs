//! Numbrion: a fast, Showdown-exact battle engine for [Gen 9] Random Doubles Battle.
//! See docs/design/ARCHITECTURE.md.

pub mod math;
pub mod prng;

pub mod actions;
pub mod battle;
pub mod dex;
pub mod effects;
pub mod event;
pub mod ids;
pub mod log;
pub mod sim;
pub mod state;
pub mod teams;
pub use battle::Battle;
