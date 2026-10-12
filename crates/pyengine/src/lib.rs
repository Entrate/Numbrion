//! Training interface for the numbrion engine: a fixed per-slot action space with exact joint legality
//! masks, hidden-information-safe observation streams, a batched, auto-resetting environment, and a
//! batched one-decision search executor (`search`).
//! The Python extension module `numbrion._numbrion` lives in `py.rs` behind the `python` feature.
//! See docs/design/TRAINING-API.md.

pub mod action;
pub mod env;
pub mod game;
pub mod mask;
pub mod pgo;
pub mod pool;
pub mod request_tree;
pub mod search;
pub mod search_grid;

#[cfg(feature = "python")]
mod py;

#[cfg(feature = "python")]
mod py_encoder;

#[cfg(feature = "python")]
mod py_search_grid;

#[cfg(all(feature = "python", feature = "search"))]
mod py_search;
