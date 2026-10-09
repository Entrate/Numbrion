//! `MockSim`: a "perfect engine" that plays a fixture back to the harness, plus fault injection
//! (`Corruption`) used to prove the harness catches each kind of divergence at the right place.

use std::sync::{Arc, OnceLock};

use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::fixture::{Fixture, SIDE_IDS, Seed, side_index};
use crate::seed;
use crate::sim::{Outcome, Rejection, Sim};

/// One injected fault. Steps are boundary indices; `end` addresses the end record where noted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Corruption {
    /// `Sim::start` fails.
    StartError,
    /// `Sim::choose` panics at this step.
    Panic {
        step: usize,
    },
    /// `Sim::choose` never returns at this step (tests the watchdog; not part of the selftest battery).
    Hang {
        step: usize,
    },
    /// Append `~` to the last token of a log line. `end` = `usize::MAX` addresses the end record.
    LogEdit {
        step: usize,
        line: usize,
    },
    LogDrop {
        step: usize,
        line: usize,
    },
    LogInsert {
        step: usize,
        line: usize,
    },
    /// Advance (n > 0) or retreat (n < 0) the PRNG state reported at this boundary.
    Seed {
        step: usize,
        draws: i32,
    },
    Turn {
        step: usize,
    },
    /// Modify the leaf at a dotted path (`side.pokemon.0.condition`) of a request.
    Request {
        step: usize,
        side: usize,
        path: String,
    },
    /// Reverse the key order of the request's top-level object (values unchanged).
    RequestOrder {
        step: usize,
        side: usize,
    },
    /// Replace the request with `{"wait":true,"side":...}`.
    RequestKind {
        step: usize,
        side: usize,
    },
    /// No request is pending for this side.
    RequestNone {
        step: usize,
        side: usize,
    },
    /// Flip accept/reject of a choice.
    ChoiceFlip {
        step: usize,
        choice: usize,
    },
    /// Change the text of a rejection.
    ChoiceError {
        step: usize,
        choice: usize,
    },
    /// Do not re-send the request after a rejection.
    ChoiceNoResend {
        step: usize,
        choice: usize,
    },
    /// Edit the request re-sent after a rejection (leaf at dotted path, default `side.id`).
    ChoiceResendEdit {
        step: usize,
        choice: usize,
    },
    /// This choice (which does not commit) emits a log line.
    ChoiceEarlyLog {
        step: usize,
        choice: usize,
    },
    /// This choice (which does not commit) draws from the PRNG.
    ChoiceEarlySeed {
        step: usize,
        choice: usize,
    },
    EndWinner,
    EndTurns,
    EndLeft,
    EndSeed,
    /// The Sim claims the battle is over at this boundary.
    EndPremature {
        step: usize,
    },
    /// The Sim never reports the end.
    EndNever,
    /// A request stays pending after the end.
    EndRequest,
}

fn parse_step(s: &str) -> Result<usize, String> {
    if s == "end" { Ok(usize::MAX) } else { s.parse().map_err(|_| format!("bad step {s:?}")) }
}
fn parse_num(s: &str) -> Result<usize, String> {
    s.parse().map_err(|_| format!("bad number {s:?}"))
}
fn parse_side(s: &str) -> Result<usize, String> {
    side_index(s).ok_or_else(|| format!("bad side {s:?} (use p1 or p2)"))
}

impl Corruption {
    pub const SYNTAX: &'static str = "\
  start-error | panic:STEP | hang:STEP | log-edit:STEP:LINE | log-drop:STEP:LINE | log-insert:STEP:LINE   (STEP may be `end`)
  seed:STEP[:DRAWS] | turn:STEP | request:STEP:SIDE[:PATH] | request-order:STEP:SIDE | request-kind:STEP:SIDE
  request-none:STEP:SIDE | choice-flip:STEP:N | choice-error:STEP:N | choice-no-resend:STEP:N
  choice-resend-edit:STEP:N[:PATH] | choice-early-log:STEP:N | choice-early-seed:STEP:N
  end-winner | end-turns | end-left | end-seed | end-premature:STEP | end-never | end-request";

    pub fn parse(spec: &str) -> Result<Corruption, String> {
        let parts: Vec<&str> = spec.split(':').collect();
        let need = |n: usize| -> Result<(), String> {
            if parts.len() < n { Err(format!("`{}` needs {} argument(s)", parts[0], n - 1)) } else { Ok(()) }
        };
        let c = match parts[0] {
            "start-error" => Corruption::StartError,
            "panic" => {
                need(2)?;
                Corruption::Panic { step: parse_num(parts[1])? }
            }
            "hang" => {
                need(2)?;
                Corruption::Hang { step: parse_num(parts[1])? }
            }
            "log-edit" | "log-drop" | "log-insert" => {
                need(3)?;
                let (step, line) = (parse_step(parts[1])?, parse_num(parts[2])?);
                match parts[0] {
                    "log-edit" => Corruption::LogEdit { step, line },
                    "log-drop" => Corruption::LogDrop { step, line },
                    _ => Corruption::LogInsert { step, line },
                }
            }
            "seed" => {
                need(2)?;
                let draws = match parts.get(2) {
                    Some(d) => d.parse().map_err(|_| format!("bad draws {d:?}"))?,
                    None => 1,
                };
                Corruption::Seed { step: parse_num(parts[1])?, draws }
            }
            "turn" => {
                need(2)?;
                Corruption::Turn { step: parse_num(parts[1])? }
            }
            "request" => {
                need(3)?;
                Corruption::Request {
                    step: parse_num(parts[1])?,
                    side: parse_side(parts[2])?,
                    path: parts.get(3).map_or("side.id".to_string(), |s| s.to_string()),
                }
            }
            "request-order" | "request-kind" | "request-none" => {
                need(3)?;
                let (step, side) = (parse_num(parts[1])?, parse_side(parts[2])?);
                match parts[0] {
                    "request-order" => Corruption::RequestOrder { step, side },
                    "request-kind" => Corruption::RequestKind { step, side },
                    _ => Corruption::RequestNone { step, side },
                }
            }
            "choice-flip" | "choice-error" | "choice-no-resend" | "choice-resend-edit" | "choice-early-log"
            | "choice-early-seed" => {
                need(3)?;
                let (step, choice) = (parse_num(parts[1])?, parse_num(parts[2])?);
                match parts[0] {
                    "choice-flip" => Corruption::ChoiceFlip { step, choice },
                    "choice-error" => Corruption::ChoiceError { step, choice },
                    "choice-no-resend" => Corruption::ChoiceNoResend { step, choice },
                    "choice-resend-edit" => Corruption::ChoiceResendEdit { step, choice },
                    "choice-early-log" => Corruption::ChoiceEarlyLog { step, choice },
                    _ => Corruption::ChoiceEarlySeed { step, choice },
                }
            }
            "end-winner" => Corruption::EndWinner,
            "end-turns" => Corruption::EndTurns,
            "end-left" => Corruption::EndLeft,
            "end-seed" => Corruption::EndSeed,
            "end-premature" => {
                need(2)?;
                Corruption::EndPremature { step: parse_num(parts[1])? }
            }
            "end-never" => Corruption::EndNever,
            "end-request" => Corruption::EndRequest,
            other => {
                return Err(format!("unknown corruption {other:?}; syntax:\n{}", Corruption::SYNTAX));
            }
        };
        Ok(c)
    }
}

/// Behavioral faults that are not expressed as edits of the fixture data.
#[derive(Clone, Debug, Default)]
struct Quirks {
    panic_step: Option<usize>,
    hang_step: Option<usize>,
    early_log: Option<(usize, usize)>,
    early_seed: Option<(usize, usize)>,
    none_requests: Vec<(usize, usize)>,
    never_ends: bool,
    premature: Option<usize>,
    request_after_end: bool,
}

fn edit_leaf(root: &mut Value, path: &str) -> bool {
    let mut cur = root;
    for seg in path.split('.') {
        let next = match cur {
            Value::Object(m) => m.get_mut(seg),
            Value::Array(a) => seg.parse::<usize>().ok().and_then(|i| a.get_mut(i)),
            _ => None,
        };
        match next {
            Some(n) => cur = n,
            None => return false,
        }
    }
    *cur = match cur {
        Value::Bool(b) => Value::Bool(!*b),
        Value::Number(n) => Value::from(n.as_i64().unwrap_or(0) + 1),
        Value::String(s) => Value::String(format!("{s}~")),
        _ => Value::String("~".into()),
    };
    true
}

fn rewrite_raw(raw: &mut Box<RawValue>, f: impl FnOnce(&mut Value) -> bool) -> bool {
    let mut v: Value = match serde_json::from_str(raw.get()) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if !f(&mut v) {
        return false;
    }
    match RawValue::from_string(serde_json::to_string(&v).expect("serializable")) {
        Ok(r) => {
            *raw = r;
            true
        }
        Err(_) => false,
    }
}

fn request_mut(fx: &mut Fixture, step: usize, side: usize) -> Option<&mut Box<RawValue>> {
    let st = fx.steps.get_mut(step)?;
    Some(if side == 0 { &mut st.requests.p1 } else { &mut st.requests.p2 })
}

fn log_mut(fx: &mut Fixture, step: usize) -> Option<&mut Vec<String>> {
    if step == usize::MAX || step == fx.steps.len() {
        Some(&mut fx.end.log)
    } else {
        fx.steps.get_mut(step).map(|s| &mut s.log)
    }
}

/// Apply `c` to a private copy of the fixture. `Err` if it does not apply (e.g. step out of range).
fn apply(c: &Corruption, fx: &mut Fixture, q: &mut Quirks) -> Result<(), String> {
    let na = |what: &str| Err(format!("corruption not applicable: {what}"));
    match c {
        Corruption::StartError => {}
        Corruption::Panic { step } | Corruption::Hang { step } => {
            if *step >= fx.steps.len() {
                return na("step out of range");
            }
            if matches!(c, Corruption::Hang { .. }) {
                q.hang_step = Some(*step);
            } else {
                q.panic_step = Some(*step);
            }
        }
        Corruption::LogEdit { step, line }
        | Corruption::LogDrop { step, line }
        | Corruption::LogInsert { step, line } => {
            let Some(log) = log_mut(fx, *step) else {
                return na("step out of range");
            };
            if *line >= log.len() {
                return na("log line out of range");
            }
            match c {
                Corruption::LogEdit { .. } => log[*line].push('~'),
                Corruption::LogDrop { .. } => {
                    log.remove(*line);
                }
                _ => log.insert(*line, "|-bogus|inserted".to_string()),
            }
        }
        Corruption::Seed { step, draws } => {
            let Some(st) = fx.steps.get_mut(*step) else {
                return na("step out of range");
            };
            for _ in 0..draws.unsigned_abs() {
                st.seed = if *draws > 0 { seed::advance(st.seed) } else { seed::retreat(st.seed) };
            }
        }
        Corruption::Turn { step } => {
            let Some(st) = fx.steps.get_mut(*step) else {
                return na("step out of range");
            };
            st.turn += 1;
        }
        Corruption::Request { step, side, path } => {
            let Some(r) = request_mut(fx, *step, *side) else {
                return na("step out of range");
            };
            if !rewrite_raw(r, |v| edit_leaf(v, path)) {
                return na("request path not found");
            }
        }
        Corruption::RequestOrder { step, side } => {
            let Some(r) = request_mut(fx, *step, *side) else {
                return na("step out of range");
            };
            let ok = rewrite_raw(r, |v| match v {
                Value::Object(m) if m.len() > 1 => {
                    let rev: Map<String, Value> = std::mem::take(m).into_iter().rev().collect();
                    *m = rev;
                    true
                }
                _ => false,
            });
            if !ok {
                return na("request has fewer than 2 keys");
            }
        }
        Corruption::RequestKind { step, side } => {
            let Some(r) = request_mut(fx, *step, *side) else {
                return na("step out of range");
            };
            let ok = rewrite_raw(r, |v| {
                let Value::Object(m) = v else { return false };
                if m.contains_key("wait") {
                    return false; // already a wait request: nothing to change
                }
                let side_v = m.get("side").cloned().unwrap_or(Value::Null);
                let mut w = Map::new();
                w.insert("wait".into(), Value::Bool(true));
                w.insert("side".into(), side_v);
                *v = Value::Object(w);
                true
            });
            if !ok {
                return na("request is already a wait request");
            }
        }
        Corruption::RequestNone { step, side } => {
            if *step >= fx.steps.len() {
                return na("step out of range");
            }
            q.none_requests.push((*step, *side));
        }
        Corruption::ChoiceFlip { step, choice }
        | Corruption::ChoiceError { step, choice }
        | Corruption::ChoiceNoResend { step, choice }
        | Corruption::ChoiceResendEdit { step, choice }
        | Corruption::ChoiceEarlyLog { step, choice }
        | Corruption::ChoiceEarlySeed { step, choice } => {
            let Some(ch) = fx.steps.get_mut(*step).and_then(|s| s.choices.get_mut(*choice)) else {
                return na("choice out of range");
            };
            match c {
                Corruption::ChoiceFlip { .. } => {
                    ch.ok = !ch.ok;
                    if !ch.ok {
                        ch.error = Some("[Invalid choice] corrupted".into());
                        ch.request = None;
                    } else {
                        ch.error = None;
                        ch.request = None;
                    }
                }
                Corruption::ChoiceError { .. } => match &mut ch.error {
                    Some(e) if !ch.ok => e.push('~'),
                    _ => return na("choice is not a rejection"),
                },
                Corruption::ChoiceNoResend { .. } => {
                    if ch.ok || ch.request.is_none() {
                        return na("choice has no re-sent request");
                    }
                    ch.request = None;
                }
                Corruption::ChoiceResendEdit { .. } => {
                    let Some(r) = ch.request.as_mut() else {
                        return na("choice has no re-sent request");
                    };
                    if !rewrite_raw(r, |v| edit_leaf(v, "side.id")) {
                        return na("request path not found");
                    }
                }
                Corruption::ChoiceEarlyLog { .. } => q.early_log = Some((*step, *choice)),
                _ => q.early_seed = Some((*step, *choice)),
            }
        }
        Corruption::EndWinner => {
            fx.end.winner = match fx.end.winner.as_deref() {
                Some("p1") => Some("p2".into()),
                _ => Some("p1".into()),
            };
        }
        Corruption::EndTurns => fx.end.turns += 1,
        Corruption::EndLeft => fx.end.pokemon_left[0] += 1,
        Corruption::EndSeed => fx.end.seed = seed::advance(fx.end.seed),
        Corruption::EndPremature { step } => {
            if *step >= fx.steps.len() {
                return na("step out of range");
            }
            q.premature = Some(*step);
        }
        Corruption::EndNever => q.never_ends = true,
        Corruption::EndRequest => q.request_after_end = true,
    }
    Ok(())
}

static GLOBAL_CORRUPTION: OnceLock<Corruption> = OnceLock::new();

/// Plays a fixture back. With a `Corruption`, deviates from it in one controlled way.
pub struct MockSim {
    fx: Arc<Fixture>,
    quirks: Quirks,
    step: usize,
    choice: usize,
    pending_log: Vec<String>,
    seed: Seed,
    turn: u32,
    outcome: Option<Outcome>,
    resent: [Option<String>; 2],
}

impl MockSim {
    /// All `Sim::start_fixture` calls (CLI `--corrupt`) apply this corruption. Set once.
    pub fn set_global_corruption(c: Corruption) {
        let _ = GLOBAL_CORRUPTION.set(c);
    }

    pub fn perfect(fx: &Arc<Fixture>) -> MockSim {
        Self::build(fx.clone(), Quirks::default())
    }

    /// A mock that misbehaves as `c` describes. `Err` if `c` does not apply to this fixture.
    pub fn corrupted(fx: &Arc<Fixture>, c: &Corruption) -> Result<MockSim, String> {
        if *c == Corruption::StartError {
            return Err("injected start failure".into());
        }
        let mut copy = (**fx).clone();
        let mut q = Quirks::default();
        apply(c, &mut copy, &mut q)?;
        Ok(Self::build(Arc::new(copy), q))
    }

    fn build(fx: Arc<Fixture>, quirks: Quirks) -> MockSim {
        let st = &fx.steps[0];
        MockSim {
            pending_log: st.log.clone(),
            seed: st.seed,
            turn: st.turn,
            quirks,
            step: 0,
            choice: 0,
            outcome: None,
            resent: [None, None],
            fx,
        }
    }

    fn advance(&mut self) {
        self.step += 1;
        self.choice = 0;
        self.resent = [None, None];
        let fx = &*self.fx;
        if self.step < fx.steps.len() {
            let st = &fx.steps[self.step];
            self.pending_log.extend(st.log.iter().cloned());
            self.seed = st.seed;
            self.turn = st.turn;
        } else {
            self.pending_log.extend(fx.end.log.iter().cloned());
            self.seed = fx.end.seed;
            self.turn = fx.end.turns;
            self.outcome = Some(Outcome {
                winner: fx.end.winner.as_deref().and_then(side_index),
                tie: fx.end.tie,
                turns: fx.end.turns,
                pokemon_left: fx.end.pokemon_left,
            });
        }
    }
}

impl Sim for MockSim {
    fn start(_seed: [u16; 4], _p1: (&str, &str), _p2: (&str, &str)) -> Result<Self, String> {
        Err("MockSim replays a whole fixture: use Sim::start_fixture".into())
    }

    fn start_fixture(fx: &Arc<Fixture>) -> Result<Self, String> {
        match GLOBAL_CORRUPTION.get() {
            Some(c) => Self::corrupted(fx, c),
            None => Ok(Self::perfect(fx)),
        }
    }

    fn seed(&self) -> [u16; 4] {
        self.seed
    }

    fn turn(&self) -> Option<u32> {
        Some(self.turn)
    }

    fn drain_log(&mut self, out: &mut Vec<String>) {
        out.append(&mut self.pending_log);
    }

    fn request_json(&self, side: usize) -> Option<String> {
        if self.outcome.is_some() {
            return self.quirks.request_after_end.then(|| "{}".to_string());
        }
        if self.quirks.none_requests.contains(&(self.step, side)) {
            return None;
        }
        if let Some(r) = &self.resent[side] {
            return Some(r.clone());
        }
        Some(self.fx.steps[self.step].requests.get(side).to_string())
    }

    fn choose(&mut self, side: usize, input: &str) -> Result<(), Rejection> {
        let err = |m: String| Err(Rejection { error: m, request: None });
        if self.outcome.is_some() {
            return err("mock: the battle is over".into());
        }
        if self.quirks.hang_step == Some(self.step) {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        if self.quirks.panic_step == Some(self.step) {
            panic!("mock: injected panic at step {}", self.step);
        }
        let fx = self.fx.clone();
        let st = &fx.steps[self.step];
        let Some(c) = st.choices.get(self.choice) else {
            return err(format!("mock: step {} has no further choices ({side}: {input})", self.step));
        };
        if side_index(&c.side) != Some(side) || c.input != input {
            return err(format!(
                "mock: unexpected choice {}: {input}, the fixture has {}: {}",
                SIDE_IDS[side.min(1)],
                c.side,
                c.input
            ));
        }
        let j = self.choice;
        self.choice += 1;
        if !c.ok {
            let rej = Rejection {
                error: c.error.clone().unwrap_or_default(),
                request: c.request.as_ref().map(|r| r.get().to_string()),
            };
            self.resent[side] = rej.request.clone();
            return Err(rej);
        }
        if self.quirks.early_log == Some((self.step, j)) {
            self.pending_log.push("|-bogus|early".into());
        }
        if self.quirks.early_seed == Some((self.step, j)) {
            self.seed = seed::advance(self.seed);
        }
        if self.choice == st.choices.len() {
            self.advance();
        }
        Ok(())
    }

    fn ended(&self) -> Option<Outcome> {
        if self.quirks.never_ends {
            return None;
        }
        if self.quirks.premature == Some(self.step) && self.outcome.is_none() {
            return Some(Outcome { winner: Some(0), tie: false, turns: self.turn, pokemon_left: [1, 0] });
        }
        self.outcome.clone()
    }
}
