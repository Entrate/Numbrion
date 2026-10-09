//! `difftest stats`: basic fixture statistics (sanity checks on a fixture file).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use serde::Serialize;

use crate::diff::line_kind;
use crate::fixture::Fixture;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub battles: u64,
    pub unusable_lines: u64,
    pub steps: u64,
    pub choices: u64,
    pub choices_rejected: u64,
    pub log_lines: u64,
    pub turns_total: u64,
    pub turns_max: u32,
    pub steps_max: u64,
    pub step_log_lines_max: u64,
    pub anomalies: u64,
    pub battles_with_anomalies: u64,
    /// Boundaries whose seed differs from the previous boundary's seed (PRNG was drawn).
    pub seed_changes: u64,
    pub step_states: BTreeMap<String, u64>,
    pub request_kinds: BTreeMap<String, u64>,
    pub winners: BTreeMap<String, u64>,
    pub rejections: BTreeMap<String, u64>,
    pub log_kinds: HashMap<String, u64>,
}

fn request_kind_of(raw: &str) -> &'static str {
    if raw.starts_with(r#"{"wait""#) {
        "wait"
    } else if raw.starts_with(r#"{"forceSwitch""#) {
        "forceSwitch"
    } else if raw.starts_with(r#"{"active""#) {
        "move"
    } else {
        "other"
    }
}

impl Stats {
    pub fn of_line(line: &str) -> Stats {
        let mut s = Stats::default();
        let fx = match Fixture::parse_line(line) {
            Ok(fx) => fx,
            Err(_) => {
                s.unusable_lines = 1;
                return s;
            }
        };
        s.battles = 1;
        s.steps = fx.steps.len() as u64;
        s.steps_max = s.steps;
        s.turns_total = fx.end.turns as u64;
        s.turns_max = fx.end.turns;
        s.anomalies = fx.anomalies.len() as u64;
        s.battles_with_anomalies = u64::from(!fx.anomalies.is_empty());
        let mut prev = fx.battle_seed;
        let count_log = |lines: &[String], s: &mut Stats| {
            s.log_lines += lines.len() as u64;
            s.step_log_lines_max = s.step_log_lines_max.max(lines.len() as u64);
            for l in lines {
                let k = line_kind(l);
                match s.log_kinds.get_mut(k) {
                    Some(c) => *c += 1,
                    None => {
                        s.log_kinds.insert(k.to_string(), 1);
                    }
                }
            }
        };
        for st in &fx.steps {
            count_log(&st.log, &mut s);
            *s.step_states.entry(st.state.clone()).or_default() += 1;
            for side in 0..2 {
                *s.request_kinds.entry(request_kind_of(st.requests.get(side)).to_string()).or_default() += 1;
            }
            if st.seed != prev {
                s.seed_changes += 1;
            }
            prev = st.seed;
            for c in &st.choices {
                s.choices += 1;
                if !c.ok {
                    s.choices_rejected += 1;
                    *s.rejections.entry(c.error.clone().unwrap_or_default()).or_default() += 1;
                }
            }
        }
        count_log(&fx.end.log, &mut s);
        let w = match (fx.end.tie, fx.end.winner.as_deref()) {
            (true, _) => "tie".to_string(),
            (_, Some(w)) => w.to_string(),
            _ => "none".to_string(),
        };
        *s.winners.entry(w).or_default() += 1;
        s
    }

    pub fn merge(&mut self, o: Stats) {
        self.battles += o.battles;
        self.unusable_lines += o.unusable_lines;
        self.steps += o.steps;
        self.choices += o.choices;
        self.choices_rejected += o.choices_rejected;
        self.log_lines += o.log_lines;
        self.turns_total += o.turns_total;
        self.turns_max = self.turns_max.max(o.turns_max);
        self.steps_max = self.steps_max.max(o.steps_max);
        self.step_log_lines_max = self.step_log_lines_max.max(o.step_log_lines_max);
        self.anomalies += o.anomalies;
        self.battles_with_anomalies += o.battles_with_anomalies;
        self.seed_changes += o.seed_changes;
        for (dst, src) in [
            (&mut self.step_states, o.step_states),
            (&mut self.request_kinds, o.request_kinds),
            (&mut self.winners, o.winners),
            (&mut self.rejections, o.rejections),
        ] {
            for (k, v) in src {
                *dst.entry(k).or_default() += v;
            }
        }
        for (k, v) in o.log_kinds {
            *self.log_kinds.entry(k).or_default() += v;
        }
    }

    pub fn render(&self, input: &str, top: usize, wall_secs: f64) -> String {
        let mut out = String::new();
        let avg = |a: u64, b: u64| if b == 0 { 0.0 } else { a as f64 / b as f64 };
        let _ = writeln!(out, "== difftest stats: {input} ==");
        let _ = writeln!(
            out,
            "battles          : {}{}",
            self.battles,
            if self.unusable_lines > 0 {
                format!("  (+{} unusable lines)", self.unusable_lines)
            } else {
                String::new()
            }
        );
        let _ = writeln!(
            out,
            "steps (boundaries): {}  (avg {:.1}/battle, max {})",
            self.steps,
            avg(self.steps, self.battles),
            self.steps_max
        );
        let _ =
            writeln!(out, "turns            : avg {:.1}, max {}", avg(self.turns_total, self.battles), self.turns_max);
        let _ = writeln!(out, "choices          : {} ({} rejected)", self.choices, self.choices_rejected);
        let _ = writeln!(
            out,
            "log lines        : {}  (avg {:.0}/battle, max {} in one step)",
            self.log_lines,
            avg(self.log_lines, self.battles),
            self.step_log_lines_max
        );
        let _ = writeln!(out, "PRNG-drawing boundaries: {} of {} steps", self.seed_changes, self.steps);
        let _ = writeln!(out, "anomalies        : {} in {} battles", self.anomalies, self.battles_with_anomalies);
        let kv = |m: &BTreeMap<String, u64>| m.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ");
        let _ = writeln!(out, "step states      : {}", kv(&self.step_states));
        let _ = writeln!(out, "request kinds    : {}", kv(&self.request_kinds));
        let _ = writeln!(out, "winners          : {}", kv(&self.winners));
        if !self.rejections.is_empty() {
            let _ = writeln!(out, "rejections:");
            let mut v: Vec<_> = self.rejections.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (k, n) in v {
                let _ = writeln!(out, "  {n:>6}  {k}");
            }
        }
        let _ = writeln!(out, "most common log message kinds ({} distinct):", self.log_kinds.len());
        let mut kinds: Vec<_> = self.log_kinds.iter().collect();
        kinds.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (k, n) in kinds.into_iter().take(top) {
            let _ = writeln!(out, "  {n:>9}  {:>5.1}%  {k}", 100.0 * *n as f64 / self.log_lines.max(1) as f64);
        }
        let _ = writeln!(out, "time             : {wall_secs:.2}s");
        out
    }
}
