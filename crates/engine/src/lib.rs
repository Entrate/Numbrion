//! Numbrion: a fast, Showdown-exact battle engine for [Gen 9] Random Doubles Battle.
//! See docs/design/ARCHITECTURE.md.

pub mod math;
pub mod prng;

pub mod ids;
pub mod dex;
pub mod state;
pub mod teams;
pub mod log;
pub mod battle;
pub use battle::Battle;
