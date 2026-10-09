//! Self-test of the harness: corrupt one aspect of a fixture in the `MockSim` and assert that the
//! replay reports exactly that divergence (category, step, signature) and nothing earlier.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::diff::{line_kind, request_kind};
use crate::fixture::Fixture;
use crate::mock::{Corruption, MockSim};
use crate::replay::{BattleResult, Category, ReplayOpts, install_panic_hook, replay_with};
use crate::seed;

/// What the harness must report for a corruption.
#[derive(Clone, Debug)]
pub struct Expect {
    pub category: Category,
    pub step: usize,
    /// The signature must start with this.
    pub sig_prefix: String,
    pub choice: Option<usize>,
    pub line: Option<usize>,
    /// Corruption is only a warning: the battle must still pass, with at least one warning.
    pub warning_only: bool,
    /// Run with `strict_order`.
    pub strict: bool,
}

#[derive(Clone, Debug)]
pub struct Case {
    pub spec: String,
    pub corruption: Corruption,
    pub expect: Expect,
}

fn exp(category: Category, step: usize, sig_prefix: impl Into<String>) -> Expect {
    Expect {
        category,
        step,
        sig_prefix: sig_prefix.into(),
        choice: None,
        line: None,
        warning_only: false,
        strict: false,
    }
}

fn parse_req(fx: &Fixture, step: usize, side: usize) -> Value {
    serde_json::from_str(fx.steps[step].requests.get(side)).unwrap()
}

/// Build the standard battery for a fixture (positions are chosen from the fixture's own data).
pub fn cases_for(fx: &Fixture) -> Vec<Case> {
    let n = fx.steps.len();
    let mut cases = Vec::new();
    let mut add = |spec: String, expect: Expect| {
        let corruption = Corruption::parse(&spec).unwrap_or_else(|e| panic!("bad spec {spec}: {e}"));
        cases.push(Case { spec, corruption, expect });
    };
    let mid = (n / 2).max(1).min(n - 1);

    add("start-error".into(), exp(Category::Start, 0, "start: injected"));
    add(format!("panic:{mid}"), exp(Category::Panic, mid, "panic"));

    // Log lines: a line with arguments whose neighbours differ (so the first mismatch is exactly there).
    let pick = |k: usize| -> Option<usize> {
        let log = fx.log_at(k);
        (0..log.len()).find(|&i| {
            let l = &log[i];
            l.split('|').count() >= 4 && (i + 1 >= log.len() || log[i + 1] != *l) && (i == 0 || log[i - 1] != *l)
        })
    };
    let log_target = (mid..=n).chain(0..mid).find_map(|k| pick(k).map(|i| (k, i)));
    if let Some((k, i)) = log_target {
        let kind = line_kind(&fx.log_at(k)[i]).to_string();
        let spec_step = if k >= n { "end".to_string() } else { k.to_string() };
        let mut e = exp(Category::Log, k, format!("log {kind}"));
        e.line = Some(i);
        add(format!("log-edit:{spec_step}:{i}"), e.clone());
        e.sig_prefix = format!("log missing {kind}");
        add(format!("log-drop:{spec_step}:{i}"), e.clone());
        e.sig_prefix = "log extra |-bogus|".into();
        add(format!("log-insert:{spec_step}:{i}"), e);
    }
    // The end record's last line.
    {
        let last = fx.end.log.len() - 1;
        let mut e = exp(Category::Log, n, "log ");
        e.line = Some(last);
        add(format!("log-edit:end:{last}"), e);
    }

    add(format!("seed:{mid}"), exp(Category::Seed, mid, "seed +1 draws"));
    add(format!("seed:{mid}:3"), exp(Category::Seed, mid, "seed +3 draws"));
    add(format!("seed:{mid}:-2"), exp(Category::Seed, mid, "seed -2 draws"));
    add("seed:0".into(), exp(Category::Seed, 0, "seed +1 draws"));
    add(format!("turn:{mid}"), exp(Category::Turn, mid, "turn"));

    add(
        format!("request:{mid}:p1:side.pokemon.0.condition"),
        exp(Category::Request, mid, "request.side.pokemon[].condition"),
    );
    add(format!("request:{mid}:p2:side.name"), exp(Category::Request, mid, "request.side.name"));
    add(format!("request-none:{mid}:p2"), exp(Category::Request, mid, "request missing"));
    // Key order: a warning by default, a failure with --strict-order.
    if let Some((k, side)) = (0..n)
        .flat_map(|k| (0..2).map(move |s| (k, s)))
        .find(|&(k, s)| k >= mid && request_kind(&parse_req(fx, k, s)) == "move")
    {
        let sid = ["p1", "p2"][side];
        let mut w = exp(Category::Request, k, "request key_order");
        w.warning_only = true;
        add(format!("request-order:{k}:{sid}"), w.clone());
        w.warning_only = false;
        w.strict = true;
        add(format!("request-order:{k}:{sid}"), w);
        let from = request_kind(&parse_req(fx, k, side));
        add(
            format!("request-kind:{k}:{sid}"),
            exp(Category::Request, k, format!("request.kind expected={from} actual=wait")),
        );
        add(
            format!("request:{k}:{sid}:active.0.moves.0.disabled"),
            exp(Category::Request, k, "request.active[].moves[].disabled"),
        );
    }

    // Choices.
    let find_choice = |pred: &dyn Fn(usize, usize, &crate::fixture::Choice, usize) -> bool| -> Option<(usize, usize)> {
        fx.steps.iter().enumerate().find_map(|(k, st)| {
            st.choices.iter().enumerate().find(|(j, c)| pred(k, *j, c, st.choices.len())).map(|(j, _)| (k, j))
        })
    };
    if let Some((k, j)) = find_choice(&|_, _, c, _| c.ok) {
        let mut e = exp(Category::Choice, k, "choice unexpected_reject");
        e.choice = Some(j);
        add(format!("choice-flip:{k}:{j}"), e);
    }
    if let Some((k, j)) = find_choice(&|_, j, c, len| c.ok && j + 1 < len) {
        let mut e = exp(Category::Choice, k, "choice log_before_commit");
        e.choice = Some(j);
        add(format!("choice-early-log:{k}:{j}"), e.clone());
        e.sig_prefix = "choice seed +1 draws".into();
        add(format!("choice-early-seed:{k}:{j}"), e);
    }
    if let Some((k, j)) = find_choice(&|_, _, c, _| !c.ok) {
        let mut e = exp(Category::Choice, k, "choice unexpected_accept");
        e.choice = Some(j);
        add(format!("choice-flip:{k}:{j}"), e.clone());
        e.sig_prefix = "choice error_text".into();
        add(format!("choice-error:{k}:{j}"), e);
    }
    if let Some((k, j)) = find_choice(&|_, _, c, _| !c.ok && c.request.is_some()) {
        let mut e = exp(Category::Choice, k, "choice.request not_resent");
        e.choice = Some(j);
        add(format!("choice-no-resend:{k}:{j}"), e.clone());
        e.sig_prefix = "choice.request.side.id".into();
        add(format!("choice-resend-edit:{k}:{j}"), e);
    }

    // End record.
    let mut e = exp(Category::End, n, "end.winner");
    add("end-winner".into(), e.clone());
    e.sig_prefix = "end.turns".into();
    add("end-turns".into(), e.clone());
    e.sig_prefix = "end.pokemon_left".into();
    add("end-left".into(), e.clone());
    e.sig_prefix = "end.seed +1 draws".into();
    add("end-seed".into(), e.clone());
    e.sig_prefix = "end not_ended".into();
    add("end-never".into(), e.clone());
    e.sig_prefix = "end.request_after_end".into();
    add("end-request".into(), e);
    add(format!("end-premature:{mid}"), exp(Category::End, mid, "end premature"));
    cases
}

/// Run one case. `Ok(())` if the harness reported exactly what was expected.
pub fn run_case(fx: &Arc<Fixture>, case: &Case) -> Result<BattleResult, String> {
    let opts = ReplayOpts { strict_order: case.expect.strict, ..ReplayOpts::default() };
    let r = replay_with::<MockSim>(0, fx, |f| MockSim::corrupted(f, &case.corruption), &opts);
    let e = &case.expect;
    if e.warning_only {
        return if r.passed() && r.warning_count > 0 && r.warnings[0].signature.starts_with(&e.sig_prefix) {
            Ok(r)
        } else {
            Err(format!(
                "expected a passing battle with a `{}` warning, got {:?} / {} warnings {:?}",
                e.sig_prefix,
                r.divergence.as_ref().map(|d| &d.signature),
                r.warning_count,
                r.warnings.first().map(|w| &w.signature)
            ))
        };
    }
    let Some(d) = &r.divergence else {
        return Err("harness did NOT notice the corruption (battle passed)".into());
    };
    if d.category != e.category {
        return Err(format!("category {:?}, expected {:?} (signature `{}`)", d.category, e.category, d.signature));
    }
    if d.step != e.step {
        return Err(format!("reported at step {}, expected step {} (signature `{}`)", d.step, e.step, d.signature));
    }
    if !d.signature.starts_with(&e.sig_prefix) {
        return Err(format!("signature `{}` does not start with `{}`", d.signature, e.sig_prefix));
    }
    if e.choice.is_some() && d.choice != e.choice {
        return Err(format!("choice {:?}, expected {:?}", d.choice, e.choice));
    }
    if e.line.is_some() && d.line != e.line {
        return Err(format!("log line {:?}, expected {:?}", d.line, e.line));
    }
    if e.category != Category::Start && e.category != Category::Panic && e.step > 0 && r.steps_ok + 1 < e.step {
        return Err(format!("only {} steps matched before a divergence at step {}", r.steps_ok, e.step));
    }
    Ok(r)
}

/// Run the whole battery on one fixture. Returns (spec, result) pairs.
pub fn run_battery(fx: &Arc<Fixture>) -> Vec<(String, Result<(), String>)> {
    install_panic_hook();
    // A perfect mock must pass cleanly.
    let mut out = Vec::new();
    let perfect = replay_with::<MockSim>(0, fx, |f| Ok(MockSim::perfect(f)), &ReplayOpts::default());
    out.push((
        "(perfect mock)".to_string(),
        if perfect.passed() && perfect.steps_ok == perfect.steps && perfect.warning_count == 0 {
            Ok(())
        } else {
            Err(format!("perfect mock did not pass: {:?}", perfect.divergence.map(|d| d.signature)))
        },
    ));
    for case in cases_for(fx) {
        let label = format!("{}{}", case.spec, if case.expect.strict { " (strict)" } else { "" });
        out.push((label, run_case(fx, &case).map(|_| ())));
    }
    out
}

// ---------------------------------------------------------------------------------------
// A small hand-built fixture so the harness can be tested without any data file
// ---------------------------------------------------------------------------------------

fn side_json(name: &str, id: &str, mon: &str, condition: &str) -> Value {
    json!({
        "name": name, "id": id,
        "pokemon": [
            {"ident": format!("{id}: {mon}"), "details": format!("{mon}, L80"), "condition": condition, "active": true,
             "stats": {"atk": 100, "def": 100, "spa": 100, "spd": 100, "spe": 100},
             "moves": ["tackle"], "baseAbility": "pressure", "item": "leftovers", "pokeball": "pokeball",
             "ability": "pressure", "commanding": false, "reviving": false, "teraType": "Normal", "terastallized": ""}
        ]
    })
}

fn move_request(name: &str, id: &str, mon: &str, condition: &str, extra: Option<(&str, Value)>) -> Value {
    let mut active = json!({"moves": [{"move": "Tackle", "id": "tackle", "pp": 56, "maxpp": 56, "target": "normal", "disabled": false}], "canTerastallize": "Normal"});
    if let Some((k, v)) = extra {
        active[k] = v;
    }
    json!({"active": [active], "side": side_json(name, id, mon, condition)})
}

fn wait_request(name: &str, id: &str, mon: &str, condition: &str) -> Value {
    json!({"wait": true, "side": side_json(name, id, mon, condition)})
}

/// A 3-step battle with a rejected-then-accepted choice, both side orders, PRNG draws and a win.
pub fn synthetic_fixture() -> Fixture {
    let s0 = [1u16, 2, 3, 4];
    let s1 = seed::advance(seed::advance(s0));
    let s2 = seed::advance(seed::advance(seed::advance(s1)));
    let s3 = seed::advance(s2);
    let blocked = move_request("Alice", "p1", "Aron", "200/200", Some(("trapped", json!(true))));
    let mut resent = blocked.clone();
    resent["update"] = json!(true);
    let v = json!({
        "v": 1, "format": "gen9randomdoublesbattle", "oracle": "synthetic", "runSeed": 0, "index": 0,
        "battleSeed": s0, "teamSeeds": [[1,1,1,1],[2,2,2,2]], "choiceSeed": [3,3,3,3],
        "players": [{"id": "p1", "name": "Alice"}, {"id": "p2", "name": "Bob"}],
        "teams": ["Aron||Leftovers|Pressure|tackle||85,85,85,85,85,85|M|||80|", "Bunnelby||Leftovers|Pickup|tackle||85,85,85,85,85,85|M|||80|"],
        "steps": [
            {
                "log": ["|t:|", "|gametype|doubles", "|player|p1|Alice||", "|player|p2|Bob||", "|gen|9", "|", "|t:|", "|start",
                        "|split|p1", "|switch|p1a: Aron|Aron, L80, M|200/200", "|switch|p1a: Aron|Aron, L80, M|100/100",
                        "|split|p2", "|switch|p2a: Bunnelby|Bunnelby, L80, M|200/200", "|switch|p2a: Bunnelby|Bunnelby, L80, M|100/100",
                        "|turn|1"],
                "turn": 1, "state": "move", "seed": s0,
                "requests": {"p1": move_request("Alice", "p1", "Aron", "200/200", None), "p2": move_request("Bob", "p2", "Bunnelby", "200/200", None)},
                "choices": [
                    {"side": "p1", "input": "move 1 1, switch 2", "ok": false, "error": "[Unavailable choice] Can't switch: The active Pokémon is trapped", "request": resent},
                    {"side": "p1", "input": "move 1 1", "ok": true},
                    {"side": "p2", "input": "move 1 1", "ok": true}
                ],
                "submitted": {"p1": "move 1 1", "p2": "move 1 1"}
            },
            {
                "log": ["|", "|t:|", "|move|p2a: Bunnelby|Tackle|p1a: Aron", "|split|p1", "|-damage|p1a: Aron|150/200", "|-damage|p1a: Aron|75/100",
                        "|move|p1a: Aron|Tackle|p2a: Bunnelby", "|split|p2", "|-damage|p2a: Bunnelby|100/200", "|-damage|p2a: Bunnelby|50/100",
                        "|upkeep", "|turn|2"],
                "turn": 2, "state": "move", "seed": s1,
                "requests": {"p1": move_request("Alice", "p1", "Aron", "150/200", None), "p2": move_request("Bob", "p2", "Bunnelby", "100/200", None)},
                "choices": [
                    {"side": "p2", "input": "move 1 1", "ok": true},
                    {"side": "p1", "input": "move 1 1", "ok": true}
                ],
                "submitted": {"p1": "move 1 1", "p2": "move 1 1"}
            },
            {
                "log": ["|", "|t:|", "|move|p1a: Aron|Tackle|p2a: Bunnelby|[miss]", "|-miss|p1a: Aron|p2a: Bunnelby",
                        "|move|p2a: Bunnelby|Tackle|p1a: Aron", "|split|p1", "|-damage|p1a: Aron|100/200", "|-damage|p1a: Aron|50/100",
                        "|upkeep", "|turn|3"],
                "turn": 3, "state": "move", "seed": s2,
                "requests": {"p1": move_request("Alice", "p1", "Aron", "100/200", None), "p2": wait_request("Bob", "p2", "Bunnelby", "100/200")},
                "choices": [{"side": "p1", "input": "move 1 1", "ok": true}],
                "submitted": {"p1": "move 1 1", "p2": null}
            }
        ],
        "end": {
            "winner": "p2", "tie": false, "turns": 3, "seed": s3, "pokemonLeft": [0, 1],
            "log": ["|", "|t:|", "|move|p2a: Bunnelby|Tackle|p1a: Aron", "|split|p1", "|-damage|p1a: Aron|0 fnt", "|-damage|p1a: Aron|0 fnt",
                    "|faint|p1a: Aron", "|", "|win|Bob"]
        }
    });
    Fixture::parse_line(&serde_json::to_string(&v).unwrap()).expect("synthetic fixture is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    fn check(label: &str, fx: &Arc<Fixture>) {
        let mut failures = Vec::new();
        let results = run_battery(fx);
        assert!(results.len() > 20, "battery too small ({})", results.len());
        for (spec, res) in results {
            if let Err(e) = res {
                failures.push(format!("{label}: {spec}: {e}"));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn battery_on_synthetic_fixture() {
        check("synthetic", &Arc::new(synthetic_fixture()));
    }

    fn sample_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/sample-50.jsonl")
    }

    #[test]
    fn battery_on_sample_fixtures() {
        let Ok(f) = std::fs::File::open(sample_path()) else {
            eprintln!("sample-50.jsonl not found, skipping");
            return;
        };
        for (i, line) in BufReader::new(f).lines().take(4).enumerate() {
            let fx = Arc::new(Fixture::parse_line(&line.unwrap()).unwrap());
            check(&format!("sample[{i}]"), &fx);
        }
    }

    #[test]
    fn perfect_mock_passes_all_samples_and_seeds_follow_the_lcg() {
        let Ok(f) = std::fs::File::open(sample_path()) else {
            return;
        };
        for (i, line) in BufReader::new(f).lines().enumerate() {
            let fx = Arc::new(Fixture::parse_line(&line.unwrap()).unwrap());
            let r = replay_with::<MockSim>(i, &fx, |f| Ok(MockSim::perfect(f)), &ReplayOpts::default());
            assert!(r.passed(), "battle {i}: {:?}", r.divergence.map(|d| d.signature));
            assert_eq!(r.steps_ok, r.steps);
            // Validates seed::relate against real data: every boundary's seed is reachable from the previous one.
            let mut prev = fx.battle_seed;
            for (k, st) in fx.steps.iter().enumerate() {
                assert!(
                    seed::draws_between(prev, st.seed, 5000).is_some(),
                    "battle {i} step {k}: seed not reachable by LCG steps"
                );
                prev = st.seed;
            }
            assert!(seed::draws_between(prev, fx.end.seed, 5000).is_some());
        }
    }

    #[test]
    fn fixture_without_anomalies_passes_and_bad_line_is_reported() {
        let r = crate::replay::replay_line::<MockSim>(3, "{not json", &ReplayOpts::default());
        let d = r.divergence.unwrap();
        assert_eq!((d.category, r.pos), (Category::Fixture, 3));
    }
}
