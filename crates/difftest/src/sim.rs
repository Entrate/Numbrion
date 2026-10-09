//! The engine-side contract. Everything the harness needs from an engine is this trait; the engine
//! adapter (`engine_sim.rs`) should be a few dozen lines. See docs/design/DIFFTEST.md.

pub use crate::fixture::Seed;
use std::sync::Arc;

use crate::fixture::Fixture;

/// Final result of a finished battle (Showdown: `battle.winner` / `battle.turn` / `side.pokemonLeft`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Winning side (0 = p1, 1 = p2); `None` for a tie.
    pub winner: Option<usize>,
    /// `|tie` ending (turn limit / forcetie). When true, `winner` is `None`.
    pub tie: bool,
    /// `battle.turn` when the battle ended.
    pub turns: u32,
    /// `side.pokemonLeft` for p1, p2.
    pub pokemon_left: [u32; 2],
}

/// A rejected `Battle.choose` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    /// The text Showdown sends after `|error|`, tag included, e.g.
    /// `[Unavailable choice] Can't switch: The active Pokémon is trapped`.
    pub error: String,
    /// The request Showdown re-sent to that side because of the rejection (JSON text with
    /// `"update":true`), or `None` if it re-sent nothing (plain `[Invalid choice]`).
    pub request: Option<String>,
}

pub trait Sim: Sized {
    /// Build the battle exactly like Showdown's `new Battle({seed})`, then `setPlayer('p1', {name,
    /// team})`, `setPlayer('p2', ...)` (which starts the battle). `p1`/`p2` are `(name, packed_team)`.
    /// After this returns the first boundary (turn 1, nothing chosen) is pending: its log lines are
    /// waiting in `drain_log` and both sides have a request.
    fn start(seed: [u16; 4], p1: (&str, &str), p2: (&str, &str)) -> Result<Self, String>;

    /// Hook for simulators that need the whole fixture (only `MockSim` does). Engines keep the default.
    fn start_fixture(fx: &Arc<Fixture>) -> Result<Self, String> {
        Self::start(fx.battle_seed, (fx.name(0), &fx.teams[0]), (fx.name(1), &fx.teams[1]))
    }

    /// Current battle PRNG state (`battle.prng.getSeed()` as four u16 words, most significant first).
    fn seed(&self) -> [u16; 4];

    /// `battle.turn` (optional extra check; `None` = not provided).
    fn turn(&self) -> Option<u32> {
        None
    }

    /// Append to `out` every log line emitted since the previous call, in Showdown's `battle.log`
    /// entry form (`|split|p1` + secret line + public line are three separate entries). `|t:|<unix>`
    /// lines may carry any timestamp or none: the harness normalizes them like the oracle does.
    fn drain_log(&mut self, out: &mut Vec<String>);

    /// The request pending for `side` (0 = p1, 1 = p2) serialized exactly as Showdown's
    /// `JSON.stringify(side.activeRequest)` (move / forceSwitch / `{"wait":true,...}`), or `None`
    /// when no request is pending (battle over).
    fn request_json(&self, side: usize) -> Option<String>;

    /// `Battle.choose(side, input)`. `Ok(())` if accepted (the choice that completes the last acting
    /// side commits the turn and runs it to the next boundary or the end), `Err` if rejected. A
    /// rejection must leave the log and PRNG untouched.
    fn choose(&mut self, side: usize, input: &str) -> Result<(), Rejection>;

    /// `Some` once the battle has ended.
    fn ended(&self) -> Option<Outcome>;
}
