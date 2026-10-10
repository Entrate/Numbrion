//! OWNER L: battle lifecycle, queue and switch/faint processing.
//! Contracts are frozen for parallel implementation; edit only this directory.

pub mod faint;
pub mod queue;
pub mod switching;
pub mod turn;
mod util;

#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod tests;

pub use queue::{ActionChoice, QueueHandle, ResolvedActions};
pub use switching::PokemonList;
pub use turn::ResidualSnapshot;
