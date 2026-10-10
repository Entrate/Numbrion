//! Training interface for the numbrion engine: a fixed per-slot action space with exact joint legality
//! masks, hidden-information-safe observation streams, and a batched, auto-resetting environment.
//! The Python extension module `numbrion._numbrion` lives in `py.rs` behind the `python` feature.
//! See docs/design/TRAINING-API.md.

pub mod action;
pub mod env;
pub mod game;
pub mod mask;
pub mod pool;

#[cfg(feature = "python")]
mod py;
