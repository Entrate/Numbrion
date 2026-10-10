//! `addMove` / `attrLastMove` / `retargetLastMove` against the real pinned Showdown: 600 random
//! op sequences (tools/probes/textlog/edit-vectors.mjs) replayed through the sink.
use super::*;

const VECTORS: &str = include_str!("edit-vectors.tsv");

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

fn ident_mon(ident: &str) -> MonId {
    match ident {
        "p1a: Wugtrio" => mon(0, 0),
        "p1b: Dondozo" => mon(0, 1),
        "p2a: Klefki" => mon(1, 0),
        "p2b: Reshiram" => mon(1, 1),
        "p1: Gholdengo" => mon(0, 2),
        "p2: Basculegion" => mon(1, 2),
        other => panic!("ident {other}"),
    }
}

fn slot_mon(slot: &str) -> MonId {
    match slot {
        "p1a" => mon(0, 0),
        "p1b" => mon(0, 1),
        "p2a" => mon(1, 0),
        "p2b" => mon(1, 1),
        other => panic!("slot {other}"),
    }
}

/// Field-position value (`side + 2 * slot`) used by the packed `MoveLineEdit::Spread`.
fn field_position(slot: &str) -> u8 {
    let b = slot.as_bytes();
    (b[1] - b'1') + 2 * (b[2] - b'a')
}

#[test]
fn random_attr_sequences_match_the_real_battle_log() {
    let mut cases = 0;
    let mut spread_variant = 0;
    for row in VECTORS.lines().filter(|l| !l.starts_with('#')) {
        let (ops, want) = row.split_once('\t').unwrap();
        // An empty log is the empty string (a lone blank entry would be `|`).
        let want: Vec<&str> = if want.is_empty() {
            vec![]
        } else {
            want.split('\u{1e}').collect()
        };
        let mut b = standard_active();
        for op in ops.split('\u{1e}') {
            let f: Vec<&str> = op.split('\u{1f}').collect();
            match f[0] {
                "plain" => {
                    let args: Vec<LogArg<'_>> = f[2..].iter().map(|a| LogArg::Text(a)).collect();
                    b.add(LogEntry::new(leak(f[1]), &args, &[]));
                }
                "move" => {
                    let tags: Vec<LogTag<'_>> = f
                        .get(4)
                        .map(|from| LogTag::Value("from", LogArg::Text(from)))
                        .into_iter()
                        .collect();
                    b.add_move(LogEntry::new(
                        "move",
                        &[LogArg::Text(f[1]), LogArg::Text(f[2]), LogArg::Text(f[3])],
                        &tags,
                    ));
                }
                "bare" => b.add_move(LogEntry::new(
                    "move",
                    &[LogArg::Text(f[1]), LogArg::Text(f[2])],
                    &[],
                )),
                "anim" => b.add_move(LogEntry::new(
                    "-anim",
                    &[LogArg::Text(f[1]), LogArg::Text(f[2]), LogArg::Text(f[3])],
                    &[],
                )),
                "retarget" => b.attr_last_move(MoveLineEdit::Retarget(ident_mon(f[1]))),
                "attr" => match f[1] {
                    "[still]" => b.attr_last_move(MoveLineEdit::Still),
                    "[miss]" => b.attr_last_move(MoveLineEdit::Miss),
                    "[notarget]" => b.attr_last_move(MoveLineEdit::NoTarget),
                    t if t.starts_with("[spread] ") => {
                        // The three equivalent encodings, rotating.
                        let slots: Vec<&str> = t["[spread] ".len()..]
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .collect();
                        spread_variant += 1;
                        match spread_variant % 3 {
                            0 => b.attr_last_move(MoveLineEdit::Tag(LogTag::Text(t))),
                            1 => {
                                let mut mons = [MonId::NONE; 4];
                                for (i, s) in slots.iter().enumerate() {
                                    mons[i] = slot_mon(s);
                                }
                                b.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
                                    "spread",
                                    LogArg::Spread {
                                        mons,
                                        len: slots.len() as u8,
                                    },
                                )));
                            }
                            _ => {
                                let mut packed = slots.len() as u8;
                                for (i, s) in slots.iter().enumerate() {
                                    packed |= field_position(s) << (2 + 2 * i);
                                }
                                b.attr_last_move(MoveLineEdit::Spread(packed));
                            }
                        }
                    }
                    t => b.attr_last_move(MoveLineEdit::Tag(LogTag::Text(t))),
                },
                other => panic!("op {other}"),
            }
        }
        assert_eq!(
            b.log.entries,
            want,
            "ops: {}",
            ops.replace('\u{1e}', " ; ").replace('\u{1f}', " ~ ")
        );
        cases += 1;
    }
    assert!(cases >= 600, "{cases}");
}
