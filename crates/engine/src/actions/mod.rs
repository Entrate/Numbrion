//! Core action APIs. The module declarations and shared types are owned centrally.
//! Move execution owns `moves/`; damage and mutations own `damage/` and `mutators/`.
pub mod damage;
pub mod moves;
pub mod mutators;
pub mod types;
pub use types::*;
