//! Replays one fixture through a [`Sim`] and reports the first divergence.
//!
//! Order of checks at every decision boundary `k` (this is the order in which the first failing
//! one is reported):
//!
//! 1. log lines produced since the previous boundary (`|t:|` normalized), exact string compare
//! 2. the battle is not over yet
//! 3. `turn` (only if the Sim provides it)
//! 4. PRNG seed
//! 5. requests p1, p2 (JSON value compare; key order is a separate warning class)
//! 6. every `choose` call in order: accept/reject, error text, re-sent request; a call that does not
//!    commit the turn must not emit log lines or draw from the PRNG
//!
//! After the last step: end log, end seed, winner/tie/turns/pokemonLeft, no pending requests.

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::diff::{self, RequestCmp, describe_json_diff, request_signature};
use crate::fixture::{Fixture, SIDE_IDS, normalize_log_line, side_index};
use crate::seed::{self, Seed, SeedRelation};
use crate::sim::{Outcome, Sim};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// The fixture line itself could not be used.
    Fixture,
    /// `Sim::start` failed.
    Start,
    /// The Sim panicked (unimplemented hook, capacity overflow, unknown id, ...).
    Panic,
    Log,
    Turn,
    Seed,
    Request,
    Choice,
    End,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Fixture => "fixture",
            Category::Start => "start",
            Category::Panic => "panic",
            Category::Log => "log",
            Category::Turn => "turn",
            Category::Seed => "seed",
            Category::Request => "request",
            Category::Choice => "choice",
            Category::End => "end",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Divergence {
    pub category: Category,
    /// Normalized grouping key, e.g. `log |-damage| arg2`, `request.active[].moves[].disabled`.
    pub signature: String,
    /// What was being checked, e.g. `log`, `request p1`, `choice 1 (p2: move 1 1)` (the step is in `step`).
    pub phase: String,
    /// Decision boundary index (`step_count` = the end record).
    pub step: usize,
    pub step_count: usize,
    /// Expected `battle.turn` at that boundary.
    pub turn: Option<u32>,
    /// Index of the `choose` call within the step, if the divergence is about a choice.
    pub choice: Option<usize>,
    pub side: Option<String>,
    pub input: Option<String>,
    /// For log divergences: index of the first mismatching line within that step's log.
    pub line: Option<usize>,
    pub message: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
    /// Unified-diff style lines (`- expected`, `+ actual`) or per-field diffs.
    pub diff: Vec<String>,
    /// The last matching log lines before the divergence (oldest first).
    pub context: Vec<String>,
    /// Inputs accepted at the previous boundary (what led here).
    pub prev_choices: Vec<String>,
    /// Seed before the choices that led to this boundary (end of previous boundary).
    pub seed_prev: Option<String>,
    pub seed_expected: Option<String>,
    pub seed_actual: Option<String>,
    pub seed_note: Option<String>,
    /// Whole expected/actual request texts (requests only, with `ReplayOpts::full`).
    pub expected_full: Option<String>,
    pub actual_full: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Warning {
    pub signature: String,
    pub step: usize,
    pub side: Option<String>,
    pub path: String,
    pub expected_order: Vec<String>,
    pub actual_order: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleResult {
    /// Position of the fixture in the input (0-based, blank lines skipped).
    pub pos: usize,
    /// The fixture's own `index` / `runSeed` fields.
    pub index: Option<u64>,
    pub run_seed: Option<i64>,
    pub battle_seed: Option<String>,
    pub steps: usize,
    pub steps_ok: usize,
    pub log_lines_ok: u64,
    pub micros: u64,
    pub warning_count: usize,
    /// First few warnings (capped).
    pub warnings: Vec<Warning>,
    pub divergence: Option<Divergence>,
}

impl BattleResult {
    pub fn passed(&self) -> bool {
        self.divergence.is_none()
    }

    pub fn fixture_error(pos: usize, message: String) -> BattleResult {
        BattleResult {
            pos,
            index: None,
            run_seed: None,
            battle_seed: None,
            steps: 0,
            steps_ok: 0,
            log_lines_ok: 0,
            micros: 0,
            warning_count: 0,
            warnings: vec![],
            divergence: Some(Divergence {
                category: Category::Fixture,
                signature: "fixture: unusable line".into(),
                phase: "parse".into(),
                step: 0,
                step_count: 0,
                turn: None,
                choice: None,
                side: None,
                input: None,
                line: None,
                message,
                expected: None,
                actual: None,
                diff: vec![],
                context: vec![],
                prev_choices: vec![],
                seed_prev: None,
                seed_expected: None,
                seed_actual: None,
                seed_note: None,
                expected_full: None,
                actual_full: None,
            }),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReplayOpts {
    /// Number of matching log lines shown before a divergence.
    pub context: usize,
    /// Treat request key-order differences as failures instead of warnings.
    pub strict_order: bool,
    /// Convert Sim panics into a `panic` divergence instead of unwinding out of the harness.
    pub catch_panics: bool,
    /// Signatures keep exact array indices (`moves[2]` instead of `moves[]`).
    pub keep_indices: bool,
    /// Attach whole expected/actual request texts to request divergences.
    pub full: bool,
}

impl Default for ReplayOpts {
    fn default() -> Self {
        ReplayOpts { context: 15, strict_order: false, catch_panics: true, keep_indices: false, full: false }
    }
}

const MAX_WARNINGS_KEPT: usize = 3;

// ---------------------------------------------------------------------------------------
// Panic capture
// ---------------------------------------------------------------------------------------

thread_local! {
    static LAST_PANIC_LOCATION: RefCell<Option<String>> = const { RefCell::new(None) };
    static IN_REPLAY: Cell<bool> = const { Cell::new(false) };
}

/// Install a panic hook that, while a Sim is being replayed, records the panic location in a
/// thread-local and stays quiet (a failing engine may panic once per battle, thousands of times).
/// Panics outside a replay (harness bugs, test assertions) go to the previous hook unchanged.
pub fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if IN_REPLAY.with(Cell::get) {
                let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line()));
                LAST_PANIC_LOCATION.with(|c| *c.borrow_mut() = loc);
            } else {
                prev(info);
            }
        }));
    });
}

fn tidy_location(loc: &str) -> String {
    match loc.find("crates/") {
        Some(p) => loc[p..].to_string(),
        None => loc.to_string(),
    }
}

fn digits_to_n(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_num = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            if !in_num {
                out.push('N');
            }
            in_num = true;
        } else {
            in_num = false;
            out.push(ch);
        }
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}...", s.chars().take(n).collect::<String>()) }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

// ---------------------------------------------------------------------------------------
// Progress (survives a panic in the Sim)
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
enum Phase {
    #[default]
    Start,
    Drain,
    Query,
    Choose {
        choice: usize,
    },
    PostChoice {
        choice: usize,
    },
    EndChecks,
}

#[derive(Default)]
struct Acc {
    step: usize,
    phase: Phase,
    steps_ok: usize,
    log_lines_ok: u64,
    warnings: Vec<Warning>,
    warning_total: usize,
}

// ---------------------------------------------------------------------------------------
// Divergence construction
// ---------------------------------------------------------------------------------------

struct Cx<'a> {
    fx: &'a Fixture,
    opts: &'a ReplayOpts,
}

#[allow(clippy::too_many_arguments)]
impl Cx<'_> {
    /// Up to `opts.context` log lines that precede line `idx` of boundary `k`'s log.
    fn context_before(&self, k: usize, idx: usize) -> Vec<String> {
        let mut chunks: Vec<&[String]> = Vec::new();
        let mut need = self.opts.context;
        let cur = self.fx.log_at(k);
        let idx = idx.min(cur.len());
        let take = idx.min(need);
        chunks.push(&cur[idx - take..idx]);
        need -= take;
        let mut kk = k;
        while need > 0 && kk > 0 {
            kk -= 1;
            let l = self.fx.log_at(kk);
            let t = l.len().min(need);
            chunks.push(&l[l.len() - t..]);
            need -= t;
        }
        chunks.iter().rev().flat_map(|c| c.iter().cloned()).collect()
    }

    fn prev_choices(&self, k: usize) -> Vec<String> {
        if k == 0 {
            return vec![];
        }
        let s = &self.fx.steps[(k - 1).min(self.fx.steps.len() - 1)].submitted;
        let mut v = Vec::new();
        if let Some(c) = &s.p1 {
            v.push(format!("p1: {c}"));
        }
        if let Some(c) = &s.p2 {
            v.push(format!("p2: {c}"));
        }
        v
    }

    fn new_div(
        &self,
        category: Category,
        signature: String,
        step: usize,
        phase: String,
        message: String,
    ) -> Divergence {
        let fx = self.fx;
        let ctx_idx = fx.log_at(step).len();
        Divergence {
            category,
            signature,
            phase,
            step,
            step_count: fx.steps.len(),
            turn: Some(fx.turn_at(step)),
            choice: None,
            side: None,
            input: None,
            line: None,
            message,
            expected: None,
            actual: None,
            diff: vec![],
            context: self.context_before(step, ctx_idx),
            prev_choices: self.prev_choices(step),
            seed_prev: if step > 0 { Some(seed::fmt(fx.seed_at(step - 1))) } else { Some(seed::fmt(fx.battle_seed)) },
            seed_expected: Some(seed::fmt(fx.seed_at(step))),
            seed_actual: None,
            seed_note: None,
            expected_full: None,
            actual_full: None,
        }
    }

    fn with_seed(&self, mut d: Divergence, actual: Option<Seed>) -> Divergence {
        if let Some(a) = actual {
            d.seed_actual = Some(seed::fmt(a));
            let exp = self.fx.seed_at(d.step);
            let rel = seed::relate(exp, a);
            d.seed_note =
                Some(if rel == SeedRelation::Equal { "seed matches expected".into() } else { rel.describe() });
        }
        d
    }

    /// Phase label; the step number is rendered separately (`Divergence::step`).
    fn step_phase(&self, step: usize, what: &str) -> String {
        if step >= self.fx.steps.len() { format!("end record {what}") } else { what.to_string() }
    }

    // ---- individual checks ----

    fn check_log(&self, step: usize, actual: &[String], sim_seed: Seed) -> Option<Divergence> {
        let expected = self.fx.log_at(step);
        let ld = diff::diff_logs(expected, actual)?;
        let mut d = self.new_div(
            Category::Log,
            ld.signature,
            step,
            self.step_phase(step, "log"),
            format!("{} [log has {} lines, expected {}]", ld.message, actual.len(), expected.len()),
        );
        d.line = Some(ld.index);
        d.expected = ld.expected_line;
        d.actual = ld.actual_line;
        d.diff = ld.rendered;
        d.context = self.context_before(step, ld.index);
        Some(self.with_seed(d, Some(sim_seed)))
    }

    fn check_seed(
        &self,
        step: usize,
        expected: Seed,
        actual: Seed,
        phase: impl FnOnce() -> String,
        category: Category,
    ) -> Option<Divergence> {
        let rel = seed::relate(expected, actual);
        let sig = match rel {
            SeedRelation::Equal => return None,
            SeedRelation::ActualAhead(n) if n <= 9 => format!("seed +{n} draws"),
            SeedRelation::ActualAhead(_) => "seed +10 or more draws".to_string(),
            SeedRelation::ActualBehind(n) if n <= 9 => format!("seed -{n} draws"),
            SeedRelation::ActualBehind(_) => "seed -10 or more draws".to_string(),
            SeedRelation::Unrelated => "seed unrelated".to_string(),
        };
        let mut d = self.new_div(category, sig, step, phase(), rel.describe());
        d.expected = Some(seed::fmt(expected));
        d.actual = Some(seed::fmt(actual));
        d.seed_actual = Some(seed::fmt(actual));
        d.seed_note = Some(rel.describe());
        Some(d)
    }

    /// Compare a request text. `prefix` is `request` or `choice.request`.
    fn check_request(
        &self,
        acc: &mut Acc,
        step: usize,
        side: usize,
        expected: &str,
        actual: Option<&str>,
        prefix: &str,
        category: Category,
        phase: impl Fn() -> String,
    ) -> Option<Divergence> {
        let side_id = SIDE_IDS[side];
        let Some(actual) = actual else {
            let mut d = self.new_div(
                category,
                format!("{prefix} missing"),
                step,
                phase(),
                format!("{side_id}: the Sim has no request but one was expected"),
            );
            d.side = Some(side_id.to_string());
            d.expected = Some(truncate(expected, 200));
            return Some(d);
        };
        match diff::compare_request(expected, actual) {
            RequestCmp::Equal => None,
            RequestCmp::OrderOnly(orders) => {
                acc.warning_total += 1;
                let o = &orders[0];
                let sig = format!(
                    "{prefix} key_order {}",
                    if o.path.is_empty() { "<root>".to_string() } else { diff::normalize_path(&o.path) }
                );
                if self.opts.strict_order {
                    let mut d = self.new_div(
                        category,
                        sig,
                        step,
                        phase(),
                        format!(
                            "{side_id}: key order differs at {:?}: expected {:?}, actual {:?}",
                            o.path, o.expected, o.actual
                        ),
                    );
                    d.side = Some(side_id.to_string());
                    d.expected = Some(o.expected.join(","));
                    d.actual = Some(o.actual.join(","));
                    return Some(d);
                }
                if acc.warnings.len() < MAX_WARNINGS_KEPT {
                    acc.warnings.push(Warning {
                        signature: sig,
                        step,
                        side: Some(side_id.to_string()),
                        path: o.path.clone(),
                        expected_order: o.expected.clone(),
                        actual_order: o.actual.clone(),
                    });
                }
                None
            }
            RequestCmp::BadJson(err) => {
                let mut d = self.new_div(
                    category,
                    format!("{prefix} invalid_json"),
                    step,
                    phase(),
                    format!("{side_id}: the Sim's request is not valid JSON: {err}"),
                );
                d.side = Some(side_id.to_string());
                d.actual = Some(truncate(actual, 200));
                Some(d)
            }
            RequestCmp::Differ(diffs) => {
                let first = &diffs[0];
                let mut d = self.new_div(
                    category,
                    request_signature(prefix, first, self.opts.keep_indices),
                    step,
                    phase(),
                    format!("{side_id}: {}", describe_json_diff(first)),
                );
                d.side = Some(side_id.to_string());
                d.expected = first.expected.clone();
                d.actual = first.actual.clone();
                d.diff = diffs.iter().map(describe_json_diff).collect();
                if self.opts.full {
                    d.expected_full = Some(expected.to_string());
                    d.actual_full = Some(actual.to_string());
                }
                Some(d)
            }
        }
    }
}

fn normalized(lines: &mut [String]) {
    for l in lines {
        normalize_log_line(l);
    }
}

fn error_head(err: &str) -> String {
    let t = err.strip_prefix("|error|").unwrap_or(err);
    match t.find(':') {
        Some(p) => t[..p].to_string(),
        None => truncate(t, 40),
    }
}

// ---------------------------------------------------------------------------------------
// The replay loop
// ---------------------------------------------------------------------------------------

fn run<S: Sim>(
    fx: &Arc<Fixture>,
    make: impl FnOnce(&Arc<Fixture>) -> Result<S, String>,
    opts: &ReplayOpts,
    acc: &mut Acc,
) -> Option<Divergence> {
    let cx = Cx { fx, opts };
    acc.phase = Phase::Start;
    let mut sim = match make(fx) {
        Ok(s) => s,
        Err(e) => {
            let mut d = cx.new_div(
                Category::Start,
                format!("start: {}", truncate(&digits_to_n(first_line(&e)), 100)),
                0,
                "Sim::start".into(),
                e.clone(),
            );
            d.context.clear();
            d.seed_prev = None;
            d.seed_expected = None;
            return Some(d);
        }
    };
    let mut scratch: Vec<String> = Vec::with_capacity(256);

    for (k, step) in fx.steps.iter().enumerate() {
        acc.step = k;

        // 1. log
        acc.phase = Phase::Drain;
        scratch.clear();
        sim.drain_log(&mut scratch);
        normalized(&mut scratch);
        if let Some(d) = cx.check_log(k, &scratch, sim.seed()) {
            return Some(d);
        }
        acc.log_lines_ok += scratch.len() as u64;

        acc.phase = Phase::Query;
        // 2. not over yet
        if let Some(o) = sim.ended() {
            let mut d = cx.new_div(
                Category::End,
                "end premature".into(),
                k,
                "battle end".to_string(),
                format!("the Sim reports the battle over ({o:?}) but the fixture has more steps"),
            );
            d.actual = Some(format!("{o:?}"));
            return Some(cx.with_seed(d, Some(sim.seed())));
        }
        // 3. turn
        if let Some(t) = sim.turn()
            && t != step.turn
        {
            let mut d = cx.new_div(
                Category::Turn,
                "turn".into(),
                k,
                cx.step_phase(k, "turn"),
                format!("battle.turn expected {}, actual {t}", step.turn),
            );
            d.expected = Some(step.turn.to_string());
            d.actual = Some(t.to_string());
            return Some(cx.with_seed(d, Some(sim.seed())));
        }
        // 4. seed
        if let Some(d) = cx.check_seed(k, step.seed, sim.seed(), || cx.step_phase(k, "seed"), Category::Seed) {
            return Some(d);
        }
        // 5. requests
        for side in 0..2 {
            let actual = sim.request_json(side);
            let phase = || cx.step_phase(k, &format!("request {}", SIDE_IDS[side]));
            if let Some(d) = cx.check_request(
                acc,
                k,
                side,
                step.requests.get(side),
                actual.as_deref(),
                "request",
                Category::Request,
                phase,
            ) {
                return Some(cx.with_seed(d, Some(sim.seed())));
            }
        }

        // 6. choices
        let n = step.choices.len();
        for (j, c) in step.choices.iter().enumerate() {
            acc.phase = Phase::Choose { choice: j };
            let side = side_index(&c.side).expect("validated");
            let res = sim.choose(side, &c.input);
            acc.phase = Phase::PostChoice { choice: j };
            let phase = || format!("choice {j} ({}: {})", c.side, c.input);
            let mk = |sig: String, msg: String| {
                let mut d = cx.new_div(Category::Choice, sig, k, phase(), msg);
                d.choice = Some(j);
                d.side = Some(c.side.clone());
                d.input = Some(c.input.clone());
                d
            };
            match (c.ok, res) {
                (true, Ok(())) => {}
                (true, Err(rej)) => {
                    let mut d = mk(
                        format!("choice unexpected_reject {}", error_head(&rej.error)),
                        format!(
                            "Showdown accepted `{}` for {} but the Sim rejected it: {}",
                            c.input, c.side, rej.error
                        ),
                    );
                    d.actual = Some(rej.error);
                    return Some(cx.with_seed(d, Some(sim.seed())));
                }
                (false, Ok(())) => {
                    let exp = c.error.clone().unwrap_or_default();
                    let mut d = mk(
                        format!("choice unexpected_accept {}", error_head(&exp)),
                        format!("Showdown rejected `{}` for {} ({exp}) but the Sim accepted it", c.input, c.side),
                    );
                    d.expected = Some(exp);
                    return Some(cx.with_seed(d, Some(sim.seed())));
                }
                (false, Err(rej)) => {
                    let exp_err = c.error.as_deref().unwrap_or("");
                    let got = rej.error.strip_prefix("|error|").unwrap_or(&rej.error);
                    if got != exp_err {
                        let mut d = mk(
                            "choice error_text".into(),
                            format!("rejection text differs: expected `{exp_err}`, actual `{got}`"),
                        );
                        d.expected = Some(exp_err.to_string());
                        d.actual = Some(got.to_string());
                        return Some(cx.with_seed(d, Some(sim.seed())));
                    }
                    match (&c.request, &rej.request) {
                        (None, None) => {}
                        (Some(_), None) => {
                            let d = mk(
                                "choice.request not_resent".into(),
                                "Showdown re-sent the request after this rejection, the Sim did not".into(),
                            );
                            return Some(cx.with_seed(d, Some(sim.seed())));
                        }
                        (None, Some(r)) => {
                            let mut d = mk(
                                "choice.request unexpected_resend".into(),
                                "the Sim re-sent a request after this rejection, Showdown did not".into(),
                            );
                            d.actual = Some(truncate(r, 200));
                            return Some(cx.with_seed(d, Some(sim.seed())));
                        }
                        (Some(exp), Some(act)) => {
                            if let Some(mut d) = cx.check_request(
                                acc,
                                k,
                                side,
                                exp.get(),
                                Some(act),
                                "choice.request",
                                Category::Choice,
                                phase,
                            ) {
                                d.choice = Some(j);
                                d.input = Some(c.input.clone());
                                return Some(cx.with_seed(d, Some(sim.seed())));
                            }
                        }
                    }
                }
            }

            // A call that does not commit the turn must not log or draw.
            let committing = c.ok && j + 1 == n;
            if !committing {
                scratch.clear();
                sim.drain_log(&mut scratch);
                if !scratch.is_empty() {
                    normalized(&mut scratch);
                    let mut d = mk(
                        "choice log_before_commit".into(),
                        format!(
                            "{} log line(s) emitted by a choice that does not commit the turn, first: {}",
                            scratch.len(),
                            scratch[0]
                        ),
                    );
                    d.actual = Some(scratch[0].clone());
                    d.diff = scratch.iter().take(10).map(|l| format!("+ {l}")).collect();
                    return Some(cx.with_seed(d, Some(sim.seed())));
                }
                if let Some(d) = cx.check_seed(k, step.seed, sim.seed(), phase, Category::Choice) {
                    let mut d = d;
                    d.signature = format!("choice {}", d.signature);
                    d.choice = Some(j);
                    d.side = Some(c.side.clone());
                    d.input = Some(c.input.clone());
                    return Some(d);
                }
            }
        }
        acc.steps_ok += 1;
    }

    // End record.
    let k = fx.steps.len();
    acc.step = k;
    acc.phase = Phase::EndChecks;
    scratch.clear();
    sim.drain_log(&mut scratch);
    normalized(&mut scratch);
    if let Some(d) = cx.check_log(k, &scratch, sim.seed()) {
        return Some(d);
    }
    acc.log_lines_ok += scratch.len() as u64;
    if let Some(d) = cx.check_seed(k, fx.end.seed, sim.seed(), || "end seed".to_string(), Category::End) {
        let mut d = d;
        d.signature = format!("end.{}", d.signature);
        return Some(d);
    }
    let Some(outcome) = sim.ended() else {
        return Some(cx.new_div(
            Category::End,
            "end not_ended".into(),
            k,
            "end record".into(),
            "the fixture battle is over but the Sim reports it still running".into(),
        ));
    };
    let expected = Outcome {
        winner: fx.end.winner.as_deref().and_then(side_index),
        tie: fx.end.tie,
        turns: fx.end.turns,
        pokemon_left: fx.end.pokemon_left,
    };
    let mismatch = |sig: &str, e: String, a: String| {
        let mut d = cx.new_div(
            Category::End,
            sig.to_string(),
            k,
            "end record".into(),
            format!("{sig}: expected {e}, actual {a}"),
        );
        d.expected = Some(e);
        d.actual = Some(a);
        d
    };
    let w = |o: Option<usize>| o.map_or("none".to_string(), |s| SIDE_IDS[s.min(1)].to_string());
    if outcome.winner != expected.winner {
        return Some(mismatch("end.winner", w(expected.winner), w(outcome.winner)));
    }
    if outcome.tie != expected.tie {
        return Some(mismatch("end.tie", expected.tie.to_string(), outcome.tie.to_string()));
    }
    if outcome.turns != expected.turns {
        return Some(mismatch("end.turns", expected.turns.to_string(), outcome.turns.to_string()));
    }
    if outcome.pokemon_left != expected.pokemon_left {
        return Some(mismatch(
            "end.pokemon_left",
            format!("{:?}", expected.pokemon_left),
            format!("{:?}", outcome.pokemon_left),
        ));
    }
    for side in 0..2 {
        if let Some(r) = sim.request_json(side) {
            let mut d = cx.new_div(
                Category::End,
                "end.request_after_end".into(),
                k,
                "end record".into(),
                format!("{} still has a pending request after the battle ended", SIDE_IDS[side]),
            );
            d.side = Some(SIDE_IDS[side].to_string());
            d.actual = Some(truncate(&r, 200));
            return Some(d);
        }
    }
    None
}

fn panic_divergence(cx: &Cx<'_>, acc: &Acc, payload: Box<dyn std::any::Any + Send>) -> Divergence {
    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    };
    let loc = LAST_PANIC_LOCATION.with(|c| c.borrow_mut().take()).map(|l| tidy_location(&l));
    let sig_msg = truncate(&digits_to_n(first_line(&msg)), 80);
    let signature = match &loc {
        Some(l) => format!("panic @{l}: {sig_msg}"),
        None => format!("panic: {sig_msg}"),
    };
    let fx = cx.fx;
    let step = acc.step.min(fx.steps.len());
    let (phase, choice) = match acc.phase {
        Phase::Start => ("panic in Sim::start".to_string(), None),
        Phase::Drain => ("panic in Sim::drain_log".to_string(), None),
        Phase::Query => ("panic in Sim::ended/turn/seed/request_json".to_string(), None),
        Phase::Choose { choice } => {
            let c = fx.steps.get(step).and_then(|s| s.choices.get(choice));
            (
                format!(
                    "panic in Sim::choose (choice {choice}: {})",
                    c.map_or(String::new(), |c| format!("{}: {}", c.side, c.input))
                ),
                Some(choice),
            )
        }
        Phase::PostChoice { choice } => (format!("panic after choice {choice}"), Some(choice)),
        Phase::EndChecks => ("panic during end-of-battle checks".to_string(), None),
    };
    let mut d = cx.new_div(
        Category::Panic,
        signature,
        step,
        phase,
        format!("{msg}{}", loc.as_ref().map_or(String::new(), |l| format!(" (at {l})"))),
    );
    d.choice = choice;
    if let Some(j) = choice
        && let Some(c) = fx.steps.get(step).and_then(|s| s.choices.get(j))
    {
        d.side = Some(c.side.clone());
        d.input = Some(c.input.clone());
    }
    d
}

/// Replay an already-parsed fixture, building the Sim with `make`.
pub fn replay_with<S: Sim>(
    pos: usize,
    fx: &Arc<Fixture>,
    make: impl FnOnce(&Arc<Fixture>) -> Result<S, String>,
    opts: &ReplayOpts,
) -> BattleResult {
    let t0 = Instant::now();
    let mut acc = Acc::default();
    let cx = Cx { fx, opts };
    let divergence = if opts.catch_panics {
        IN_REPLAY.with(|c| c.set(true));
        let caught = catch_unwind(AssertUnwindSafe(|| run(fx, make, opts, &mut acc)));
        IN_REPLAY.with(|c| c.set(false));
        match caught {
            Ok(d) => d,
            Err(payload) => Some(panic_divergence(&cx, &acc, payload)),
        }
    } else {
        run(fx, make, opts, &mut acc)
    };
    BattleResult {
        pos,
        index: Some(fx.index),
        run_seed: Some(fx.run_seed),
        battle_seed: Some(seed::fmt(fx.battle_seed)),
        steps: fx.steps.len(),
        steps_ok: acc.steps_ok,
        log_lines_ok: acc.log_lines_ok,
        micros: t0.elapsed().as_micros() as u64,
        warning_count: acc.warning_total,
        warnings: acc.warnings,
        divergence,
    }
}

/// Parse a fixture line and replay it through `S::start_fixture`.
pub fn replay_line<S: Sim>(pos: usize, line: &str, opts: &ReplayOpts) -> BattleResult {
    let t0 = Instant::now();
    let mut r = replay_line_untimed::<S>(pos, line, opts);
    r.micros = t0.elapsed().as_micros() as u64;
    r
}

fn replay_line_untimed<S: Sim>(pos: usize, line: &str, opts: &ReplayOpts) -> BattleResult {
    match Fixture::parse_line(line) {
        Ok(fx) => {
            let fx = Arc::new(fx);
            let mut r = replay_with::<S>(pos, &fx, S::start_fixture, opts);
            if !fx.anomalies.is_empty() && r.warnings.len() < MAX_WARNINGS_KEPT {
                r.warning_count += 1;
                r.warnings.push(Warning {
                    signature: "fixture has anomalies".into(),
                    step: 0,
                    side: None,
                    path: String::new(),
                    expected_order: vec![],
                    actual_order: fx.anomalies.iter().take(3).cloned().collect(),
                });
            }
            r
        }
        Err(e) => BattleResult::fixture_error(pos, e),
    }
}
