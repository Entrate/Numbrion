//! Pure helpers: the JS result algebra, rounding and clamping, against vectors recorded from the
//! pinned `BattleActions.combineResults`, `Math.round` and `Battle.clampIntRange`.
use super::super::support::*;
use crate::event::Relay;

fn decode(s: &str) -> Relay {
    match s {
        "U" => Relay::Undefined,
        "N" => Relay::Null,
        "S" => Relay::NotFail,
        "T" => Relay::Bool(true),
        "F" => Relay::Bool(false),
        n => Relay::Number(num(n)),
    }
}
fn num(s: &str) -> f64 {
    match s.strip_prefix('#').expect("number") {
        "NaN" => f64::NAN,
        "-0" => -0.0,
        n => n.parse().unwrap(),
    }
}
fn encode(r: Relay) -> String {
    match r {
        Relay::Undefined => "U".into(),
        Relay::Null => "N".into(),
        Relay::NotFail => "S".into(),
        Relay::Bool(true) => "T".into(),
        Relay::Bool(false) => "F".into(),
        Relay::Number(n) => encode_num(n),
        other => panic!("{other:?}"),
    }
}
/// Bit-identical, or both NaN (signed zero is observable in JS via Object.is).
fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}
fn encode_num(n: f64) -> String {
    if n.is_nan() {
        "#NaN".into()
    } else if n == 0.0 && n.is_sign_negative() {
        "#-0".into()
    } else {
        format!("#{n}")
    }
}

#[test]
fn result_algebra_rounding_and_clamping_match_pinned_showdown() {
    let mut counts = [0usize; 3];
    for line in include_str!("pure-vectors.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<&str> = line.split('\t').collect();
        match c[0] {
            "C" => {
                assert_eq!(
                    encode(combine_results(decode(c[1]), decode(c[2]))),
                    c[4],
                    "{line}"
                );
                counts[0] += 1;
            }
            "J" => {
                assert!(same(js_round(num(c[1])), num(c[4])), "{line}");
                counts[1] += 1;
            }
            "K" => {
                let bound = |s: &str| (s != "-").then(|| s.parse::<f64>().unwrap());
                assert!(
                    same(
                        clamp_int_range(num(c[1]), bound(c[2]), bound(c[3])),
                        num(c[4])
                    ),
                    "{line}"
                );
                counts[2] += 1;
            }
            other => panic!("unknown vector {other}"),
        }
    }
    assert_eq!(counts, [144, 24, 52]);
}

#[test]
fn survivor_and_failure_predicates_follow_js_truthiness() {
    use Relay::*;
    // `hitResults[i] || hitResults[i] === 0` keeps zero and truthy values only.
    for (r, kept) in [
        (Bool(true), true),
        (Number(0.0), true),
        (Number(-0.0), true),
        (Number(7.0), true),
        (Bool(false), false),
        (Null, false),
        (Undefined, false),
        (NotFail, false),
        (Number(f64::NAN), false),
    ] {
        assert_eq!(keeps(r), kept, "{r:?}");
        assert_eq!(failed_not_zero(r), !kept, "{r:?}");
    }
    assert!(is_false(Bool(false)));
    assert!(!is_false(Null) && !is_false(Number(0.0)) && !is_false(Undefined));
}
