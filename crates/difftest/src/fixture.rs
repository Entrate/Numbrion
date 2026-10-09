//! Typed view of one fixture line (docs/design/FIXTURES.md, format version 1).
//!
//! Requests are kept as raw JSON text (`Box<RawValue>`): the fast path of the comparison is a plain
//! string compare against what the engine serialized, and key order is preserved exactly as Showdown
//! wrote it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub use crate::seed::Seed;

pub const SUPPORTED_VERSION: u32 = 1;
pub const SUPPORTED_FORMAT: &str = "gen9randomdoublesbattle";
pub const SIDE_IDS: [&str; 2] = ["p1", "p2"];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Fixture {
    pub v: u32,
    pub format: String,
    #[serde(default)]
    pub oracle: String,
    #[serde(rename = "runSeed", default)]
    pub run_seed: i64,
    #[serde(default)]
    pub index: u64,
    #[serde(rename = "battleSeed")]
    pub battle_seed: Seed,
    pub players: Vec<Player>,
    pub teams: Vec<String>,
    pub steps: Vec<Step>,
    pub end: End,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anomalies: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Player {
    pub id: String,
    pub name: String,
}

/// One decision boundary.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Step {
    /// `battle.log` entries appended since the previous boundary (already `|t:|`-normalized).
    pub log: Vec<String>,
    pub turn: u32,
    /// `battle.requestState`: "move" or "switch".
    pub state: String,
    pub seed: Seed,
    pub requests: Requests,
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub submitted: Submitted,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Requests {
    pub p1: Box<RawValue>,
    pub p2: Box<RawValue>,
}

impl Requests {
    pub fn get(&self, side: usize) -> &str {
        match side {
            0 => self.p1.get(),
            _ => self.p2.get(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Choice {
    pub side: String,
    pub input: String,
    pub ok: bool,
    /// Text after `|error|` (includes the `[Invalid choice]` / `[Unavailable choice]` tag).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The request Showdown re-sent after a hidden-information rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Box<RawValue>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Submitted {
    #[serde(default)]
    pub p1: Option<String>,
    #[serde(default)]
    pub p2: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct End {
    pub winner: Option<String>,
    pub tie: bool,
    pub turns: u32,
    pub seed: Seed,
    #[serde(rename = "pokemonLeft")]
    pub pokemon_left: [u32; 2],
    pub log: Vec<String>,
}

/// "p1" -> 0, "p2" -> 1.
pub fn side_index(id: &str) -> Option<usize> {
    SIDE_IDS.iter().position(|s| *s == id)
}

impl Fixture {
    pub fn parse_line(line: &str) -> Result<Fixture, String> {
        let fx: Fixture = serde_json::from_str(line).map_err(|e| format!("invalid fixture JSON: {e}"))?;
        fx.validate()?;
        Ok(fx)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.v != SUPPORTED_VERSION {
            return Err(format!("unsupported fixture version {} (expected {SUPPORTED_VERSION})", self.v));
        }
        if self.format != SUPPORTED_FORMAT {
            return Err(format!("unsupported format {:?} (expected {SUPPORTED_FORMAT})", self.format));
        }
        if self.players.len() != 2 || self.teams.len() != 2 {
            return Err("fixture must have exactly 2 players and 2 teams".into());
        }
        if self.players[0].id != "p1" || self.players[1].id != "p2" {
            return Err("fixture players must be [p1, p2]".into());
        }
        if self.steps.is_empty() {
            return Err("fixture has no steps".into());
        }
        for (k, st) in self.steps.iter().enumerate() {
            for (j, c) in st.choices.iter().enumerate() {
                if side_index(&c.side).is_none() {
                    return Err(format!("step {k} choice {j}: unknown side {:?}", c.side));
                }
            }
        }
        Ok(())
    }

    pub fn name(&self, side: usize) -> &str {
        &self.players[side].name
    }

    /// Log lines of boundary `k`; `k == steps.len()` is the end record's final delta.
    pub fn log_at(&self, k: usize) -> &[String] {
        if k < self.steps.len() { &self.steps[k].log } else { &self.end.log }
    }

    /// `battle.turn` at boundary `k` (the end record's turn count for `k == steps.len()`).
    pub fn turn_at(&self, k: usize) -> u32 {
        if k < self.steps.len() { self.steps[k].turn } else { self.end.turns }
    }

    /// PRNG seed at boundary `k` (`end.seed` for `k == steps.len()`).
    pub fn seed_at(&self, k: usize) -> Seed {
        if k < self.steps.len() { self.steps[k].seed } else { self.end.seed }
    }
}

/// `^\|t:\|\d+$` -> `|t:|`, exactly the oracle's normalization (tools/oracle/lib/common.mjs).
pub fn normalize_log_line(line: &mut String) {
    if line.len() > 4 && line.starts_with("|t:|") && line.as_bytes()[4..].iter().all(u8::is_ascii_digit) {
        line.truncate(4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_normalization() {
        let mut a = "|t:|1700000000".to_string();
        normalize_log_line(&mut a);
        assert_eq!(a, "|t:|");
        for keep in ["|t:|", "|t:|12x", "|t:|-5", "|turn|3", "|t:|1 "] {
            let mut s = keep.to_string();
            normalize_log_line(&mut s);
            assert_eq!(s, keep);
        }
    }
}
