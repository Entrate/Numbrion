//! State-dependent strings against the pinned Showdown: `Pokemon.toString`, `getUpdatedDetails`,
//! `(illusion || this).details + tera`, `getHealth` (both views) and `getFullDetails`.
use super::*;
use crate::log::format::{write_details, write_health, write_ident};

/// Applies one probe row's state to the engine (the mirror of `row()` in state-vectors.mjs) and
/// returns the mon.
fn apply(b: &mut Battle<TextLog>, spec: &str) -> MonId {
    let mut m = None;
    let mut act = None;
    let mut hp = None;
    let mut st = Status::None;
    let mut ill = None;
    let mut tera = None;
    let mut sp = None;
    let mon_ref = |r: &str| {
        let (s, i) = r.split_once(':').unwrap();
        mon(s.parse().unwrap(), i.parse().unwrap())
    };
    for kv in spec.split(' ') {
        let (k, v) = kv.split_once('=').unwrap();
        match k {
            "m" => m = Some(mon_ref(v)),
            "act" => act = Some(if v == "a" { 0 } else { 1 }),
            "hp" => hp = Some(v.parse::<u16>().unwrap()),
            "st" => st = status_of(v),
            "ill" => ill = Some(mon_ref(v)),
            "tera" => tera = Some(dex::type_id(v).unwrap()),
            "sp" => sp = Some(id(EffectKind::Species, v)),
            _ => panic!("spec key {k}"),
        }
    }
    let m = m.unwrap();
    let p = &mut b.state.pokemon[m.0 as usize];
    // Every row starts from a bench mon at full HP; rows are independent.
    p.flags &= !mon_flags::ACTIVE;
    if let Some(slot) = act {
        p.flags |= mon_flags::ACTIVE;
        p.position = slot;
    }
    if let Some(hp) = hp {
        p.hp = hp;
    }
    p.status = st;
    if let Some(i) = ill {
        p.illusion = i;
    }
    if let Some(t) = tera {
        p.terastallized = t;
    }
    if let Some(s) = sp {
        p.species = s;
        p.base_species = s;
    }
    m
}

fn string(f: impl FnOnce(&mut String)) -> String {
    let mut s = String::new();
    f(&mut s);
    s
}

#[test]
fn ident_details_and_health_match_pinned_showdown() {
    let mut b = standard();
    let mut rows = 0;
    for line in STATE_VECTORS.lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<&str> = line.split('\t').collect();
        assert_eq!(c.len(), 8, "{line}");
        let pristine = b.state.pokemon;
        let m = apply(&mut b, c[0]);
        let v = view(&b);
        let ctx = c[0];
        assert_eq!(string(|s| write_ident(s, v, m)), c[1], "ident {ctx}");
        assert_eq!(
            string(|s| write_details(s, v, m, true)),
            c[2],
            "details (illusion || this).details + tera: {ctx}"
        );
        assert_eq!(
            string(|s| write_details(s, v, m, false)),
            c[3],
            "getUpdatedDetails: {ctx}"
        );
        assert_eq!(string(|s| write_health(s, v, m, true)), c[4], "secret hp {ctx}");
        assert_eq!(string(|s| write_health(s, v, m, false)), c[5], "shared hp {ctx}");
        // The function part getFullDetails, via the public argument writer, in both views.
        for (secret, want) in [(true, c[6]), (false, c[7])] {
            let got = string(|s| {
                crate::log::format::write_arg(s, v, LogArg::FullDetails(m), secret);
            });
            assert_eq!(got, want, "getFullDetails secret={secret} {ctx}");
        }
        b.state.pokemon = pristine;
        rows += 1;
    }
    assert!(rows > 1000, "expected the full vector set, got {rows}");
}

/// The same rows through the sink: a `switch` split triple built from `Mon` + `FullDetails`, a
/// `detailschange` (`Details`, illusion/tera aware) and a `replace` (plain `getUpdatedDetails`).
#[test]
fn switch_detailschange_and_replace_lines_match_pinned_showdown() {
    let mut b = standard();
    for line in STATE_VECTORS.lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<&str> = line.split('\t').collect();
        let pristine = b.state.pokemon;
        let m = apply(&mut b, c[0]);
        let side = m.side().0;
        let got = add_split(
            &mut b,
            "switch",
            &[LogArg::Mon(m), LogArg::FullDetails(m)],
            &[],
            side,
            false,
        );
        assert_eq!(got[0], format!("|split|p{}", side + 1), "{}", c[0]);
        assert_eq!(got[1], format!("|switch|{}|{}", c[1], c[6]), "{}", c[0]);
        assert_eq!(got[2], format!("|switch|{}|{}", c[1], c[7]), "{}", c[0]);
        let got = add(&mut b, "detailschange", &[LogArg::Mon(m), LogArg::Details(m)], &[]);
        assert_eq!(got, [format!("|detailschange|{}|{}", c[1], c[2])], "{}", c[0]);
        let got = add(&mut b, "replace", &[LogArg::Mon(m), LogArg::Details(m)], &[]);
        assert_eq!(got, [format!("|replace|{}|{}", c[1], c[3])], "{}", c[0]);
        b.state.pokemon = pristine;
        b.log.entries.clear();
    }
}

#[test]
fn health_rule_edge_cases_by_hand() {
    // The 99 clamp, ceil, tiny HP, fainted, status on both views (hand-derived from pokemon.ts:2075-2104).
    let mut b = standard_active();
    let m = mon(0, 0); // Wugtrio, 214 HP
    for (hp, secret, shared) in [
        (214, "214/214", "100/100"),
        (213, "213/214", "99/100"),
        (212, "212/214", "99/100"),
        (211, "211/214", "99/100"),
        (1, "1/214", "1/100"),
        (0, "0 fnt", "0 fnt"),
        (107, "107/214", "50/100"),
        (108, "108/214", "51/100"),
    ] {
        b.state.pokemon[m.0 as usize].hp = hp;
        let v = view(&b);
        assert_eq!(string(|s| write_health(s, v, m, true)), secret);
        assert_eq!(string(|s| write_health(s, v, m, false)), shared);
    }
    // `0 fnt` never carries a status; everything else does, on both views, `tox` stays `tox`.
    b.state.pokemon[m.0 as usize].status = Status::Toxic;
    b.state.pokemon[m.0 as usize].hp = 0;
    assert_eq!(string(|s| write_health(s, view(&b), m, true)), "0 fnt");
    b.state.pokemon[m.0 as usize].hp = 100;
    assert_eq!(string(|s| write_health(s, view(&b), m, true)), "100/214 tox");
    assert_eq!(string(|s| write_health(s, view(&b), m, false)), "47/100 tox");
}

#[test]
fn ident_uses_set_name_never_the_forme() {
    let b = standard_active();
    // p1:3 is Deoxys-Defense (set name "Deoxys"); p2:2 is Basculegion-F; p2:4 Ogerpon-Wellspring.
    for (m, want) in [
        (mon(0, 3), "p1: Deoxys"),
        (mon(1, 2), "p2: Basculegion"),
        (mon(1, 4), "p2: Ogerpon"),
        (mon(1, 3), "p2: Arceus"),
    ] {
        assert_eq!(string(|s| write_ident(s, view(&b), m)), want);
    }
}
