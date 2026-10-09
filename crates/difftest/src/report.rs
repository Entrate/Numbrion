//! Aggregation of per-battle results into the failure-signature report that drives the bug-fix
//! loop, plus the human and JSON renderings.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::replay::{BattleResult, Category, Divergence};

#[derive(Clone, Debug, Serialize)]
pub struct Example {
    pub pos: usize,
    pub step: usize,
    pub turn: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SigStat {
    pub signature: String,
    pub category: Category,
    pub count: usize,
    /// Smallest repros first (lowest step, then position), at most `MAX_EXAMPLES`.
    pub examples: Vec<Example>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CategoryStat {
    pub category: Category,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct WarningStat {
    pub signature: String,
    /// Number of battles that logged this warning (only the first few per battle are recorded).
    pub battles: usize,
    pub examples: Vec<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub input: String,
    pub sim: String,
    pub jobs: usize,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub battles_with_warnings: usize,
    pub steps_total: u64,
    pub steps_matched: u64,
    pub log_lines_matched: u64,
    pub wall_secs: f64,
    pub cpu_secs: f64,
    pub battles_per_sec: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    /// Schema version of this JSON document.
    pub version: u32,
    pub summary: Summary,
    pub categories: Vec<CategoryStat>,
    pub signatures: Vec<SigStat>,
    pub warnings: Vec<WarningStat>,
    /// Every failing battle (sorted by position), with its first divergence.
    pub failures: Vec<BattleResult>,
}

const MAX_EXAMPLES: usize = 5;

#[derive(Default)]
pub struct Aggregator {
    total: usize,
    passed: usize,
    warned: usize,
    steps_total: u64,
    steps_ok: u64,
    log_lines: u64,
    cpu_micros: u64,
    failures: Vec<BattleResult>,
    warn: HashMap<String, (usize, Vec<usize>)>,
}

impl Aggregator {
    pub fn add(&mut self, r: BattleResult) {
        self.total += 1;
        self.steps_total += r.steps as u64;
        self.steps_ok += r.steps_ok as u64;
        self.log_lines += r.log_lines_ok;
        self.cpu_micros += r.micros;
        if r.warning_count > 0 {
            self.warned += 1;
            let mut seen = Vec::new();
            for w in &r.warnings {
                if seen.contains(&&w.signature) {
                    continue;
                }
                seen.push(&w.signature);
                let e = self.warn.entry(w.signature.clone()).or_default();
                e.0 += 1;
                if e.1.len() < MAX_EXAMPLES {
                    e.1.push(r.pos);
                }
            }
        }
        if r.passed() {
            self.passed += 1;
        } else {
            self.failures.push(r);
        }
    }

    pub fn failed(&self) -> usize {
        self.failures.len()
    }

    pub fn finish(mut self, input: &str, sim: &str, jobs: usize, wall_secs: f64) -> Report {
        self.failures.sort_by_key(|r| r.pos);
        let mut by_sig: HashMap<&str, (Category, usize, Vec<Example>)> = HashMap::new();
        let mut by_cat: HashMap<Category, usize> = HashMap::new();
        for r in &self.failures {
            let d = r.divergence.as_ref().expect("failure has a divergence");
            *by_cat.entry(d.category).or_default() += 1;
            let e = by_sig.entry(d.signature.as_str()).or_insert((d.category, 0, Vec::new()));
            e.1 += 1;
            e.2.push(Example { pos: r.pos, step: d.step, turn: d.turn });
        }
        let mut signatures: Vec<SigStat> = by_sig
            .into_iter()
            .map(|(sig, (category, count, mut ex))| {
                ex.sort_by_key(|e| (e.step, e.pos));
                ex.truncate(MAX_EXAMPLES);
                SigStat { signature: sig.to_string(), category, count, examples: ex }
            })
            .collect();
        signatures.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.signature.cmp(&b.signature)));
        let mut categories: Vec<CategoryStat> =
            by_cat.into_iter().map(|(category, count)| CategoryStat { category, count }).collect();
        categories.sort_by(|a, b| b.count.cmp(&a.count).then(a.category.cmp(&b.category)));
        let mut warnings: Vec<WarningStat> = self
            .warn
            .into_iter()
            .map(|(signature, (battles, examples))| WarningStat { signature, battles, examples })
            .collect();
        warnings.sort_by(|a, b| b.battles.cmp(&a.battles).then_with(|| a.signature.cmp(&b.signature)));
        let failed = self.failures.len();
        Report {
            version: 1,
            summary: Summary {
                input: input.to_string(),
                sim: sim.to_string(),
                jobs,
                total: self.total,
                passed: self.passed,
                failed,
                battles_with_warnings: self.warned,
                steps_total: self.steps_total,
                steps_matched: self.steps_ok,
                log_lines_matched: self.log_lines,
                wall_secs,
                cpu_secs: self.cpu_micros as f64 / 1e6,
                battles_per_sec: if wall_secs > 0.0 { self.total as f64 / wall_secs } else { 0.0 },
            },
            categories,
            signatures,
            warnings,
            failures: self.failures,
        }
    }
}

fn pct(a: u64, b: u64) -> f64 {
    if b == 0 { 100.0 } else { 100.0 * a as f64 / b as f64 }
}

pub struct RenderOpts {
    /// Number of failing battles shown in detail.
    pub show: usize,
    /// Only show failures whose signature contains this text.
    pub sig_filter: Option<String>,
    /// Command prefix used for the "repro" line.
    pub repro_cmd: String,
    /// Number of signature rows in the summary table.
    pub top: usize,
}

pub fn render_divergence(out: &mut String, r: &BattleResult, d: &Divergence, repro_cmd: &str) {
    let _ = writeln!(
        out,
        "---- FAIL battle {} (fixture index {}, run seed {}, battle seed {}) ----",
        r.pos,
        r.index.map_or("?".to_string(), |v| v.to_string()),
        r.run_seed.map_or("?".to_string(), |v| v.to_string()),
        r.battle_seed.as_deref().unwrap_or("?"),
    );
    let _ = writeln!(out, "  signature : {}", d.signature);
    let at = match d.category {
        Category::Fixture | Category::Start => d.phase.clone(),
        _ if d.step >= d.step_count => format!("{} (after the last step)", d.phase),
        _ => format!(
            "{}, step {} (of {}){}{}",
            d.phase,
            d.step,
            d.step_count,
            d.turn.map_or(String::new(), |t| format!(", turn {t}")),
            d.line.map_or(String::new(), |l| format!(", log line {l}")),
        ),
    };
    let _ = writeln!(out, "  where     : {at}");
    let _ = writeln!(
        out,
        "  progress  : {}/{} steps matched before this, {} log lines",
        r.steps_ok, r.steps, r.log_lines_ok
    );
    if !d.prev_choices.is_empty() {
        let _ = writeln!(out, "  led by    : {}", d.prev_choices.join("  |  "));
    }
    if matches!(d.category, Category::Fixture | Category::Start) {
        // no PRNG state exists yet
    } else if d.seed_expected.is_some() || d.seed_actual.is_some() {
        let _ = writeln!(
            out,
            "  seeds     : previous boundary {} | expected {} | actual {}",
            d.seed_prev.as_deref().unwrap_or("-"),
            d.seed_expected.as_deref().unwrap_or("-"),
            d.seed_actual.as_deref().unwrap_or("-"),
        );
        if let Some(n) = &d.seed_note {
            let _ = writeln!(out, "              {n}");
        }
    }
    let _ = writeln!(out, "  message   : {}", d.message);
    if matches!(d.category, Category::Seed | Category::Request | Category::Choice | Category::End | Category::Turn) {
        if let Some(e) = &d.expected {
            let _ = writeln!(out, "  expected  : {e}");
        }
        if let Some(a) = &d.actual {
            let _ = writeln!(out, "  actual    : {a}");
        }
    }
    if !d.context.is_empty() {
        let _ = writeln!(out, "  last {} matching log lines:", d.context.len());
        for l in &d.context {
            let _ = writeln!(out, "      {l}");
        }
    }
    if d.diff.len() > 1 || (!d.diff.is_empty() && d.category == Category::Log) {
        let head = if d.category == Category::Log { "log diff (- expected, + actual)" } else { "differences" };
        let _ = writeln!(out, "  {head}:");
        for l in &d.diff {
            let _ = writeln!(out, "      {l}");
        }
    }
    if let (Some(e), Some(a)) = (&d.expected_full, &d.actual_full) {
        let _ = writeln!(out, "  expected request: {e}");
        let _ = writeln!(out, "  actual request  : {a}");
    }
    if !r.warnings.is_empty() {
        let _ =
            writeln!(out, "  warnings before the failure: {} (first: {})", r.warning_count, r.warnings[0].signature);
    }
    let _ = writeln!(out, "  repro     : {repro_cmd} --battle {}", r.pos);
    out.push('\n');
}

pub fn render_human(rep: &Report, ro: &RenderOpts) -> String {
    let s = &rep.summary;
    let mut out = String::new();
    let _ = writeln!(out, "== difftest replay: {} (sim: {}, {} jobs) ==", s.input, s.sim, s.jobs);

    let shown: Vec<&BattleResult> = rep
        .failures
        .iter()
        .filter(|r| {
            ro.sig_filter
                .as_ref()
                .is_none_or(|f| r.divergence.as_ref().is_some_and(|d| d.signature.contains(f.as_str())))
        })
        .take(ro.show)
        .collect();
    for r in &shown {
        render_divergence(&mut out, r, r.divergence.as_ref().unwrap(), &ro.repro_cmd);
    }
    let matching = rep
        .failures
        .iter()
        .filter(|r| {
            ro.sig_filter
                .as_ref()
                .is_none_or(|f| r.divergence.as_ref().is_some_and(|d| d.signature.contains(f.as_str())))
        })
        .count();
    if matching > shown.len() {
        let _ = writeln!(
            out,
            "... {} more failing battle(s) not shown (use --show N, --sig TEXT, or --json)\n",
            matching - shown.len()
        );
    }

    let _ = writeln!(
        out,
        "battles : {} total, {} passed, {} failed ({:.1}% pass){}",
        s.total,
        s.passed,
        s.failed,
        pct(s.passed as u64, s.total as u64),
        if s.battles_with_warnings > 0 {
            format!(", {} with warnings", s.battles_with_warnings)
        } else {
            String::new()
        }
    );
    let _ = writeln!(
        out,
        "progress: {}/{} steps matched ({:.1}%), {} log lines matched",
        s.steps_matched,
        s.steps_total,
        pct(s.steps_matched, s.steps_total),
        s.log_lines_matched
    );
    let _ =
        writeln!(out, "time    : {:.2}s wall, {:.2}s cpu, {:.0} battles/s", s.wall_secs, s.cpu_secs, s.battles_per_sec);

    if !rep.categories.is_empty() {
        let _ = writeln!(out, "\nfirst-divergence categories:");
        for c in &rep.categories {
            let _ = writeln!(out, "  {:>6}  {}", c.count, c.category.as_str());
        }
        let _ = writeln!(out, "\nfailure signatures by frequency (repro = battle@step, smallest first):");
        let width = rep.signatures.iter().take(ro.top).map(|s| s.signature.chars().count()).max().unwrap_or(0).min(70);
        for sig in rep.signatures.iter().take(ro.top) {
            let ex: Vec<String> = sig.examples.iter().map(|e| format!("{}@{}", e.pos, e.step)).collect();
            let _ = writeln!(out, "  {:>6}  {:<width$}  {}", sig.count, sig.signature, ex.join(" "), width = width);
        }
        if rep.signatures.len() > ro.top {
            let _ = writeln!(out, "  ... {} more signatures (--top N or --json)", rep.signatures.len() - ro.top);
        }
    }
    if !rep.warnings.is_empty() {
        let _ = writeln!(out, "\nwarnings (do not fail a battle):");
        for w in &rep.warnings {
            let ex: Vec<String> = w.examples.iter().map(|e| e.to_string()).collect();
            let _ = writeln!(out, "  {:>6} battles  {}  (e.g. battle {})", w.battles, w.signature, ex.join(", "));
        }
    }
    if s.failed == 0 {
        let _ = writeln!(out, "\nALL {} BATTLES PASSED", s.total);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Corruption, MockSim};
    use crate::replay::{ReplayOpts, replay_with};
    use crate::selftest::synthetic_fixture;
    use std::sync::Arc;

    fn run(pos: usize, spec: Option<&str>) -> BattleResult {
        let fx = Arc::new(synthetic_fixture());
        match spec {
            None => replay_with::<MockSim>(pos, &fx, |f| Ok(MockSim::perfect(f)), &ReplayOpts::default()),
            Some(s) => {
                let c = Corruption::parse(s).unwrap();
                replay_with::<MockSim>(pos, &fx, |f| MockSim::corrupted(f, &c), &ReplayOpts::default())
            }
        }
    }

    #[test]
    fn signatures_are_ranked_by_frequency_with_smallest_repros_first() {
        let mut agg = Aggregator::default();
        agg.add(run(0, None));
        agg.add(run(1, Some("seed:2")));
        agg.add(run(2, Some("end-winner")));
        agg.add(run(3, Some("seed:1")));
        agg.add(run(4, Some("end-winner")));
        agg.add(run(5, Some("seed:2")));
        agg.add(run(6, Some("log-edit:1:2")));
        agg.add(run(7, Some("request-order:1:p1"))); // warning only
        let rep = agg.finish("mem", "mock", 1, 1.0);
        assert_eq!((rep.summary.total, rep.summary.passed, rep.summary.failed), (8, 2, 6));
        assert_eq!(rep.summary.battles_with_warnings, 1);
        let sigs: Vec<(&str, usize)> = rep.signatures.iter().map(|s| (s.signature.as_str(), s.count)).collect();
        assert_eq!(sigs[0], ("seed +1 draws", 3));
        assert_eq!(sigs[1], ("end.winner", 2));
        assert_eq!(sigs[2].1, 1);
        // Smallest step first: seed:1 (pos 3) fails at step 1, seed:2 (pos 1, 5) at step 2.
        let seed = &rep.signatures[0];
        assert_eq!(seed.examples.iter().map(|e| (e.pos, e.step)).collect::<Vec<_>>(), vec![(3, 1), (1, 2), (5, 2)]);
        assert_eq!(rep.failures.iter().map(|r| r.pos).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(rep.warnings[0].signature, "request key_order <root>");
        let text = render_human(
            &rep,
            &RenderOpts { show: 1, sig_filter: None, repro_cmd: "difftest replay mem".into(), top: 10 },
        );
        assert!(text.contains("seed +1 draws") && text.contains("failure signatures by frequency"), "{text}");
        let json = serde_json::to_string(&rep).unwrap();
        assert!(json.contains("\"signatures\"") && json.contains("\"failures\""));
    }
}
