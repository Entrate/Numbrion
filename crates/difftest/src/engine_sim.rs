//! Engine adapter: connects `engine::Battle<TextLog>` to the harness. See docs/design/DIFFTEST.md.

use crate::sim::{Outcome, Rejection, Sim};
use engine::log::text::TextLog;

pub struct EngineSim {
    battle: engine::Battle<TextLog>,
}

impl Sim for EngineSim {
    fn start(seed: [u16; 4], p1: (&str, &str), p2: (&str, &str)) -> Result<Self, String> {
        let mut battle = engine::Battle::from_players(seed, p1, p2, TextLog::default())
            .map_err(|e| format!("team error: {e}"))?;
        battle.start().map_err(|e| format!("start error: {e}"))?;
        Ok(Self { battle })
    }

    fn seed(&self) -> [u16; 4] {
        self.battle.seed()
    }

    fn turn(&self) -> Option<u32> {
        Some(self.battle.turn())
    }

    fn drain_log(&mut self, out: &mut Vec<String>) {
        self.battle.drain_log(out);
    }

    fn request_json(&self, side: usize) -> Option<String> {
        self.battle.request_json(side)
    }

    fn choose(&mut self, side: usize, input: &str) -> Result<(), Rejection> {
        self.battle.choose(side, input).map_err(|e| Rejection {
            error: e.text,
            request: e.resent_request_json,
        })
    }

    fn ended(&self) -> Option<Outcome> {
        self.battle.outcome().map(|o| Outcome {
            winner: o.winner,
            tie: o.tie,
            turns: o.turns,
            pokemon_left: o.pokemon_left,
        })
    }
}
