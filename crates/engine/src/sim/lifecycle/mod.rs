//! OWNER L: battle lifecycle, queue and switch/faint processing.
//! Contracts are frozen for parallel implementation; edit only this directory.

pub mod faint;
pub mod queue;
pub mod switching;
pub mod turn;

pub use queue::{ActionChoice, QueueHandle, ResolvedActions};
pub use switching::PokemonList;
pub use turn::ResidualSnapshot;
