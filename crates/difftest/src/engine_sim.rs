//! ============================================================================================
//! ENGINE ADAPTER: STUB. THE ENGINE AUTHOR FILLS THIS IN. See docs/design/DIFFTEST.md.
//! ============================================================================================
//!
//! `EngineSim` connects `engine::Battle` to the harness. Today the engine can only construct a
//! battle (`engine::Battle::new`, no start/turn loop/choices/requests), so `start` builds the
//! battle to prove the wiring compiles and then reports "engine not ready"; every failing battle
//! then shows up as the single signature `start: engine not ready ...`.
//!
//! To implement, replace the body of each method below. The engine needs to provide (see the doc):
//!   * start with player names (`|player|p1|Alice||`, `|win|Bob` are part of the log),
//!   * a text log sink that collects Showdown-form `battle.log` entries,
//!   * the current PRNG seed, `turn`, the pending request JSON per side, `choose(side, input)`
//!     returning the `|error|` text and the re-sent request, and the final outcome.

use crate::sim::{Outcome, Rejection, Sim};

pub struct EngineSim {
    // e.g. battle: engine::Battle<TextLog>, plus a cursor into the collected log text.
    _battle: engine::Battle,
}

const NOT_READY: &str =
    "engine not ready: no turn loop / choices / requests yet (fill in crates/difftest/src/engine_sim.rs)";

impl Sim for EngineSim {
    fn start(seed: [u16; 4], p1: (&str, &str), p2: (&str, &str)) -> Result<Self, String> {
        // TODO(engine): construct the battle with player names + text log, run start, and return Ok(Self{..}).
        let _battle = engine::Battle::new(seed, p1.1, p2.1).map_err(|e| format!("team error: {e}"))?;
        let _ = (p1.0, p2.0);
        Err(NOT_READY.to_string())
    }

    fn seed(&self) -> [u16; 4] {
        todo!("EngineSim::seed: battle PRNG state as four u16 words")
    }

    fn turn(&self) -> Option<u32> {
        None // TODO(engine): Some(battle turn) enables the extra turn check
    }

    fn drain_log(&mut self, _out: &mut Vec<String>) {
        todo!("EngineSim::drain_log: append log entries emitted since the last call")
    }

    fn request_json(&self, _side: usize) -> Option<String> {
        todo!("EngineSim::request_json: pending request of the side as Showdown JSON")
    }

    fn choose(&mut self, _side: usize, _input: &str) -> Result<(), Rejection> {
        todo!("EngineSim::choose: Battle.choose(side, input); Err(Rejection {{ error, request }}) on rejection")
    }

    fn ended(&self) -> Option<Outcome> {
        todo!("EngineSim::ended: Some(Outcome) once the battle is over")
    }
}
