//! Synchronous simulation boundary. Lifecycle and choice owners edit separate dirs.
pub mod choices;
pub mod lifecycle;
pub use choices::{ChoiceError, LegalActions};
pub use lifecycle::queue::{ActionChoice, QueueHandle, ResolvedActions};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleError(pub String);
impl core::fmt::Display for BattleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for BattleError {}
/// End record required by DIFFTEST.md; ties have no winner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub winner: Option<usize>,
    pub tie: bool,
    pub turns: u32,
    pub pokemon_left: [u32; 2],
}
