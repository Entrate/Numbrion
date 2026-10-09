//! Diffing and signature building: log lines (LCS based) and JSON requests (structural).
//!
//! A *signature* is a short, stable string describing the kind of the first mismatch, used to group
//! failures across battles (e.g. `log |-damage| arg2`, `request.active[].moves[].disabled`).

use serde_json::Value;

// ---------------------------------------------------------------------------------------
// Log lines
// ---------------------------------------------------------------------------------------

/// Message kind of a protocol line: `|move|a|b` -> `|move|`, `|upkeep` -> `|upkeep`, `|` -> `|`.
pub fn line_kind(line: &str) -> &str {
    match line.strip_prefix('|') {
        Some(rest) => match rest.find('|') {
            Some(p) => &line[..p + 2],
            None => line,
        },
        None if line.is_empty() => "(empty)",
        None => {
            let end = line.char_indices().nth(20).map_or(line.len(), |(i, _)| i);
            &line[..end]
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkKind {
    /// Expected line(s) absent from the actual log.
    Missing,
    /// Actual log has line(s) that are not expected.
    Extra,
    /// Same position, different content.
    Replace,
}

#[derive(Clone, Debug)]
pub struct LogDiff {
    /// Index (in the compared log) of the first mismatching line.
    pub index: usize,
    pub hunk: HunkKind,
    pub signature: String,
    /// One-line human description.
    pub message: String,
    /// Unified-diff style lines (`- expected`, `+ actual`, `  same`), starting at the mismatch.
    pub rendered: Vec<String>,
    pub expected_line: Option<String>,
    pub actual_line: Option<String>,
}

#[derive(Clone, Copy)]
enum Op {
    Eq,
    Del(usize),
    Ins(usize),
}

const WINDOW: usize = 40;
const RENDER_LIMIT: usize = 14;

fn edit_script(e: &[String], a: &[String]) -> Vec<Op> {
    let (n, m) = (e.len(), a.len());
    let w = m + 1;
    let mut dp = vec![0u16; (n + 1) * w];
    for x in (0..n).rev() {
        for y in (0..m).rev() {
            dp[x * w + y] =
                if e[x] == a[y] { dp[(x + 1) * w + y + 1] + 1 } else { dp[(x + 1) * w + y].max(dp[x * w + y + 1]) };
        }
    }
    let (mut x, mut y) = (0, 0);
    let mut ops = Vec::new();
    while x < n && y < m {
        if e[x] == a[y] {
            ops.push(Op::Eq);
            x += 1;
            y += 1;
        } else if dp[(x + 1) * w + y] >= dp[x * w + y + 1] {
            ops.push(Op::Del(x));
            x += 1;
        } else {
            ops.push(Op::Ins(y));
            y += 1;
        }
    }
    while x < n {
        ops.push(Op::Del(x));
        x += 1;
    }
    while y < m {
        ops.push(Op::Ins(y));
        y += 1;
    }
    ops
}

/// Compare two logs (already normalized). `None` if identical.
pub fn diff_logs(exp: &[String], act: &[String]) -> Option<LogDiff> {
    let i = exp.iter().zip(act).take_while(|(a, b)| a == b).count();
    if i == exp.len() && i == act.len() {
        return None;
    }
    let e = &exp[i..(i + WINDOW).min(exp.len())];
    let a = &act[i..(i + WINDOW).min(act.len())];
    let ops = edit_script(e, a);

    // First hunk: consecutive non-Eq ops from the start.
    let mut dels = Vec::new();
    let mut ins = Vec::new();
    for op in &ops {
        match op {
            Op::Eq => break,
            Op::Del(x) => dels.push(*x),
            Op::Ins(y) => ins.push(*y),
        }
    }

    let (hunk, signature, message, expected_line, actual_line) = match (dels.first(), ins.first()) {
        (Some(&dx), None) => {
            let l = &e[dx];
            (
                HunkKind::Missing,
                format!("log missing {}", line_kind(l)),
                format!("expected line is missing from actual: {l}"),
                Some(l.clone()),
                a.first().cloned(),
            )
        }
        (None, Some(&iy)) => {
            let l = &a[iy];
            (
                HunkKind::Extra,
                format!("log extra {}", line_kind(l)),
                format!("actual has an unexpected line: {l}"),
                e.first().cloned(),
                Some(l.clone()),
            )
        }
        (Some(&dx), Some(&iy)) => {
            let (sig, msg) = token_diff(&e[dx], &a[iy]);
            (HunkKind::Replace, format!("log {sig}"), msg, Some(e[dx].clone()), Some(a[iy].clone()))
        }
        (None, None) => unreachable!("lines differ so the first op cannot be Eq"),
    };

    let mut rendered = Vec::new();
    let (mut x, mut y) = (0usize, 0usize);
    for op in &ops {
        if rendered.len() >= RENDER_LIMIT {
            rendered.push("  ...".to_string());
            break;
        }
        match op {
            Op::Eq => {
                rendered.push(format!("  {}", e[x]));
                x += 1;
                y += 1;
            }
            Op::Del(_) => {
                rendered.push(format!("- {}", e[x]));
                x += 1;
            }
            Op::Ins(_) => {
                rendered.push(format!("+ {}", a[y]));
                y += 1;
            }
        }
    }

    Some(LogDiff { index: i, hunk, signature, message, rendered, expected_line, actual_line })
}

fn tag_name(tok: &str) -> &str {
    match tok.find(']') {
        Some(p) => &tok[..=p],
        None => tok,
    }
}

/// Token-wise comparison of two lines that sit at the same position. Returns (signature, message).
fn token_diff(exp: &str, act: &str) -> (String, String) {
    let et: Vec<&str> = exp.split('|').collect();
    let at: Vec<&str> = act.split('|').collect();
    let ek = line_kind(exp);
    let ak = line_kind(act);
    if ek != ak {
        return (format!("expected {ek} got {ak}"), format!("expected a {ek} line but actual has {ak}"));
    }
    // Same kind: first differing token (tokens[0] is the empty text before the leading '|', tokens[1] the kind).
    let j = (0..et.len().max(at.len())).find(|&j| et.get(j) != at.get(j)).unwrap_or(0);
    let arg = j.saturating_sub(1);
    let (te, ta) = (et.get(j).copied(), at.get(j).copied());
    let sig_tail = match (te, ta) {
        (None, Some(t)) if t.starts_with('[') => format!("+tag{}", tag_name(t)),
        (None, Some(_)) => format!("+arg{arg}"),
        (Some(t), None) if t.starts_with('[') => format!("-tag{}", tag_name(t)),
        (Some(_), None) => format!("-arg{arg}"),
        (Some(x), Some(y)) if x.starts_with('[') || y.starts_with('[') => {
            let (nx, ny) = (tag_name(x), tag_name(y));
            if x.starts_with('[') && y.starts_with('[') && nx == ny {
                format!("tag{nx}")
            } else {
                format!("tag{nx} vs tag{ny}")
            }
        }
        _ => format!("arg{arg}"),
    };
    let msg =
        format!("{ek} {}: expected `{}`, actual `{}`", sig_tail, te.unwrap_or("<absent>"), ta.unwrap_or("<absent>"));
    (format!("{ek} {sig_tail}"), msg)
}

// ---------------------------------------------------------------------------------------
// JSON (requests)
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Changed,
    /// Present in expected, absent in actual.
    Missing,
    /// Present in actual, absent in expected.
    Extra,
    /// Array length differs.
    Len,
}

#[derive(Clone, Debug)]
pub struct JsonDiff {
    /// Exact path, e.g. `active[0].moves[2].disabled` (empty = the root).
    pub path: String,
    pub kind: DiffKind,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

#[derive(Clone, Debug)]
pub struct OrderDiff {
    pub path: String,
    pub expected: Vec<String>,
    pub actual: Vec<String>,
}

/// `a.b[3].c` -> `a.b[].c`.
pub fn normalize_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut skipping = false;
    for ch in p.chars() {
        match ch {
            '[' => {
                out.push('[');
                skipping = true;
            }
            ']' => {
                out.push(']');
                skipping = false;
            }
            _ if skipping => {}
            c => out.push(c),
        }
    }
    out
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 160 {
        let mut end = 160;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    } else {
        s
    }
}

/// Which kind of request this is.
pub fn request_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Object(m) if m.contains_key("wait") => "wait",
        Value::Object(m) if m.contains_key("forceSwitch") => "forceSwitch",
        Value::Object(m) if m.contains_key("active") => "move",
        _ => "unknown",
    }
}

/// Structural diff in expected-key order (then actual-only keys). At most `limit` entries.
pub fn json_diffs(exp: &Value, act: &Value, limit: usize) -> Vec<JsonDiff> {
    let mut out = Vec::new();
    let mut path = String::new();
    walk(exp, act, &mut path, &mut out, limit);
    out
}

fn walk(exp: &Value, act: &Value, path: &mut String, out: &mut Vec<JsonDiff>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    match (exp, act) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                if out.len() >= limit {
                    return;
                }
                let n = path.len();
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(k);
                match a.get(k) {
                    Some(av) => walk(ev, av, path, out, limit),
                    None => out.push(JsonDiff {
                        path: path.clone(),
                        kind: DiffKind::Missing,
                        expected: Some(short(ev)),
                        actual: None,
                    }),
                }
                path.truncate(n);
            }
            for (k, av) in a {
                if out.len() >= limit {
                    return;
                }
                if !e.contains_key(k) {
                    let n = path.len();
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(k);
                    out.push(JsonDiff {
                        path: path.clone(),
                        kind: DiffKind::Extra,
                        expected: None,
                        actual: Some(short(av)),
                    });
                    path.truncate(n);
                }
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            let common = e.len().min(a.len());
            for i in 0..common {
                if out.len() >= limit {
                    return;
                }
                let n = path.len();
                path.push_str(&format!("[{i}]"));
                walk(&e[i], &a[i], path, out, limit);
                path.truncate(n);
            }
            if e.len() != a.len() && out.len() < limit {
                out.push(JsonDiff {
                    path: path.clone(),
                    kind: DiffKind::Len,
                    expected: Some(format!("len {}", e.len())),
                    actual: Some(format!("len {}", a.len())),
                });
            }
        }
        _ => {
            if exp != act {
                out.push(JsonDiff {
                    path: path.clone(),
                    kind: DiffKind::Changed,
                    expected: Some(short(exp)),
                    actual: Some(short(act)),
                });
            }
        }
    }
}

/// Objects whose keys are the same but serialized in a different order (values assumed equal).
pub fn order_diffs(exp: &Value, act: &Value, limit: usize) -> Vec<OrderDiff> {
    let mut out = Vec::new();
    let mut path = String::new();
    order_walk(exp, act, &mut path, &mut out, limit);
    out
}

fn order_walk(exp: &Value, act: &Value, path: &mut String, out: &mut Vec<OrderDiff>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    match (exp, act) {
        (Value::Object(e), Value::Object(a)) => {
            let ek: Vec<&String> = e.keys().collect();
            let ak: Vec<&String> = a.keys().collect();
            if ek != ak {
                out.push(OrderDiff {
                    path: path.clone(),
                    expected: ek.iter().map(|s| s.to_string()).collect(),
                    actual: ak.iter().map(|s| s.to_string()).collect(),
                });
            }
            for (k, ev) in e {
                if let Some(av) = a.get(k) {
                    let n = path.len();
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(k);
                    order_walk(ev, av, path, out, limit);
                    path.truncate(n);
                }
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            for i in 0..e.len().min(a.len()) {
                let n = path.len();
                path.push_str(&format!("[{i}]"));
                order_walk(&e[i], &a[i], path, out, limit);
                path.truncate(n);
            }
        }
        _ => {}
    }
}

/// Outcome of comparing two request JSON texts.
#[derive(Clone, Debug)]
pub enum RequestCmp {
    Equal,
    /// Values equal, key order differs (warning class).
    OrderOnly(Vec<OrderDiff>),
    Differ(Vec<JsonDiff>),
    /// The actual text is not valid JSON.
    BadJson(String),
}

pub fn compare_request(expected: &str, actual: &str) -> RequestCmp {
    if expected == actual {
        return RequestCmp::Equal;
    }
    let e: Value = match serde_json::from_str(expected) {
        Ok(v) => v,
        Err(err) => return RequestCmp::BadJson(format!("(fixture side) {err}")),
    };
    let a: Value = match serde_json::from_str(actual) {
        Ok(v) => v,
        Err(err) => return RequestCmp::BadJson(err.to_string()),
    };
    let (ek, ak) = (request_kind(&e), request_kind(&a));
    if ek != ak {
        return RequestCmp::Differ(vec![JsonDiff {
            path: "kind".into(),
            kind: DiffKind::Changed,
            expected: Some(ek.to_string()),
            actual: Some(ak.to_string()),
        }]);
    }
    let d = json_diffs(&e, &a, 12);
    if !d.is_empty() {
        return RequestCmp::Differ(d);
    }
    let o = order_diffs(&e, &a, 6);
    if o.is_empty() { RequestCmp::Equal } else { RequestCmp::OrderOnly(o) }
}

/// Signature for the first request diff. `keep_indices` keeps exact array indices.
pub fn request_signature(prefix: &str, d: &JsonDiff, keep_indices: bool) -> String {
    let p = if keep_indices { d.path.clone() } else { normalize_path(&d.path) };
    if d.path == "kind" && d.kind == DiffKind::Changed {
        return format!(
            "{prefix}.kind expected={} actual={}",
            d.expected.as_deref().unwrap_or("?"),
            d.actual.as_deref().unwrap_or("?")
        );
    }
    let root = if p.is_empty() { prefix.to_string() } else { format!("{prefix}.{p}") };
    match d.kind {
        DiffKind::Changed => root,
        DiffKind::Missing => format!("{root} (missing)"),
        DiffKind::Extra => format!("{root} (unexpected)"),
        DiffKind::Len => format!("{root} (length)"),
    }
}

pub fn describe_json_diff(d: &JsonDiff) -> String {
    let at = if d.path.is_empty() { "<root>" } else { d.path.as_str() };
    match d.kind {
        DiffKind::Changed => format!(
            "{at}: expected {}, actual {}",
            d.expected.as_deref().unwrap_or("?"),
            d.actual.as_deref().unwrap_or("?")
        ),
        DiffKind::Missing => format!("{at}: missing in actual (expected {})", d.expected.as_deref().unwrap_or("?")),
        DiffKind::Extra => format!("{at}: unexpected in actual (value {})", d.actual.as_deref().unwrap_or("?")),
        DiffKind::Len => {
            format!("{at}: array {} vs {}", d.expected.as_deref().unwrap_or("?"), d.actual.as_deref().unwrap_or("?"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn kinds() {
        assert_eq!(line_kind("|move|p1a: X|Y"), "|move|");
        assert_eq!(line_kind("|upkeep"), "|upkeep");
        assert_eq!(line_kind("|"), "|");
        assert_eq!(line_kind(""), "(empty)");
        assert_eq!(line_kind("|t:|"), "|t:|");
    }

    #[test]
    fn equal_logs() {
        assert!(diff_logs(&v(&["|a|1", "|b"]), &v(&["|a|1", "|b"])).is_none());
    }

    #[test]
    fn replace_arg() {
        let d = diff_logs(&v(&["|x", "|-damage|p1a: A|53/100", "|y"]), &v(&["|x", "|-damage|p1a: A|57/100", "|y"]))
            .unwrap();
        assert_eq!(d.index, 1);
        assert_eq!(d.hunk, HunkKind::Replace);
        assert_eq!(d.signature, "log |-damage| arg2");
    }

    #[test]
    fn replace_kind_and_tags() {
        let d = diff_logs(&v(&["|-damage|p1a: A|5/100"]), &v(&["|-heal|p1a: A|5/100"])).unwrap();
        assert_eq!(d.signature, "log expected |-damage| got |-heal|");
        let d = diff_logs(&v(&["|move|a|b|c|[miss]"]), &v(&["|move|a|b|c"])).unwrap();
        assert_eq!(d.signature, "log |move| -tag[miss]");
        let d =
            diff_logs(&v(&["|-damage|a|1/2|[from] item: Life Orb"]), &v(&["|-damage|a|1/2|[from] item: Leftovers"]))
                .unwrap();
        assert_eq!(d.signature, "log |-damage| tag[from]");
        let d = diff_logs(&v(&["|-damage|a|1/2"]), &v(&["|-damage|a|1/2|[from] item: Life Orb"])).unwrap();
        assert_eq!(d.signature, "log |-damage| +tag[from]");
    }

    #[test]
    fn missing_and_extra() {
        let d = diff_logs(&v(&["|a", "|-miss|x", "|b", "|c"]), &v(&["|a", "|b", "|c"])).unwrap();
        assert_eq!((d.index, d.hunk, d.signature.as_str()), (1, HunkKind::Missing, "log missing |-miss|"));
        let d = diff_logs(&v(&["|a", "|b", "|c"]), &v(&["|a", "|-crit|x", "|b", "|c"])).unwrap();
        assert_eq!((d.index, d.hunk, d.signature.as_str()), (1, HunkKind::Extra, "log extra |-crit|"));
        // actual ends early
        let d = diff_logs(&v(&["|a", "|turn|2"]), &v(&["|a"])).unwrap();
        assert_eq!((d.index, d.hunk, d.signature.as_str()), (1, HunkKind::Missing, "log missing |turn|"));
        // actual longer
        let d = diff_logs(&v(&["|a"]), &v(&["|a", "|win|Bob"])).unwrap();
        assert_eq!((d.index, d.hunk, d.signature.as_str()), (1, HunkKind::Extra, "log extra |win|"));
        assert!(d.rendered.iter().any(|l| l == "+ |win|Bob"));
    }

    #[test]
    fn json_paths_and_order() {
        let e: Value = serde_json::from_str(
            r#"{"active":[{"moves":[{"id":"a","disabled":false},{"id":"b","disabled":false}]}],"side":{"id":"p1"}}"#,
        )
        .unwrap();
        let a: Value = serde_json::from_str(
            r#"{"active":[{"moves":[{"id":"a","disabled":false},{"id":"b","disabled":true}]}],"side":{"id":"p1"}}"#,
        )
        .unwrap();
        let d = json_diffs(&e, &a, 10);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path, "active[0].moves[1].disabled");
        assert_eq!(request_signature("request", &d[0], false), "request.active[].moves[].disabled");
        assert_eq!(request_signature("request", &d[0], true), "request.active[0].moves[1].disabled");

        let a2: Value = serde_json::from_str(
            r#"{"side":{"id":"p1"},"active":[{"moves":[{"disabled":false,"id":"a"},{"id":"b","disabled":false}]}]}"#,
        )
        .unwrap();
        assert!(json_diffs(&e, &a2, 10).is_empty());
        let o = order_diffs(&e, &a2, 10);
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].path, "");
        assert_eq!(o[1].path, "active[0].moves[0]");
    }

    #[test]
    fn request_compare_classes() {
        let e = r#"{"wait":true,"side":{"id":"p1"}}"#;
        assert!(matches!(compare_request(e, e), RequestCmp::Equal));
        assert!(matches!(compare_request(e, r#"{ "wait": true, "side": { "id": "p1" } }"#), RequestCmp::Equal));
        assert!(matches!(compare_request(e, r#"{"side":{"id":"p1"},"wait":true}"#), RequestCmp::OrderOnly(_)));
        match compare_request(e, r#"{"active":[],"side":{"id":"p1"}}"#) {
            RequestCmp::Differ(d) => {
                assert_eq!(request_signature("request", &d[0], false), "request.kind expected=wait actual=move")
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(compare_request(e, "{nope"), RequestCmp::BadJson(_)));
        match compare_request(r#"{"a":[1,2]}"#, r#"{"a":[1]}"#) {
            RequestCmp::Differ(d) => assert_eq!(request_signature("request", &d[0], false), "request.a (length)"),
            other => panic!("{other:?}"),
        }
    }
}
