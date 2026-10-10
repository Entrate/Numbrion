//! Logical unsent-line counting (LINE LIMIT guard input), drain cursor, hints, opening log, and
//! NoLog/TextLog parity.
use super::*;

fn pair() -> (Battle<NoLog>, Battle<TextLog>) {
    let (names, teams) = standard_teams();
    let quiet = Battle::from_players(
        [1, 2, 3, 4],
        (&names[0], &teams[0]),
        (&names[1], &teams[1]),
        NoLog,
    )
    .unwrap();
    (quiet, standard())
}

#[test]
fn pre_start_count_is_four_and_the_opening_log_realizes_it_once() {
    let (mut quiet, mut text) = pair();
    assert_eq!(quiet.scratch.unsent_lines, 4);
    assert_eq!(text.scratch.unsent_lines, 4);
    assert!(
        text.log.entries.is_empty(),
        "nothing is written before start"
    );

    quiet.emit_opening_log();
    text.emit_opening_log();
    assert_eq!(
        quiet.scratch.unsent_lines, 4,
        "reset before emitting: not double counted"
    );
    assert_eq!(text.scratch.unsent_lines, 4);
    assert_eq!(
        lines(&text),
        [
            "|t:|",
            "|gametype|doubles",
            "|player|p1|Alice||",
            "|player|p2|Bob||"
        ]
    );
    // Both counters keep agreeing as lines are added.
    for b in [
        &mut text as &mut dyn Counter,
        &mut quiet as &mut dyn Counter,
    ] {
        b.line();
    }
    assert_eq!(quiet.scratch.unsent_lines, 5);
    assert_eq!(text.scratch.unsent_lines, 5);
}

/// Adds one `|-fail|` line to either sink instantiation.
trait Counter {
    fn line(&mut self);
}
impl<L: LogSink> Counter for Battle<L> {
    fn line(&mut self) {
        self.add(LogEntry::new("-fail", &[LogArg::Mon(MonId(0))], &[]));
    }
}

#[test]
fn custom_player_names_reach_the_opening_log() {
    let (_, teams) = standard_teams();
    let mut b = Battle::from_players(
        [1, 2, 3, 4],
        ("Zoë Ünï", &teams[0]),
        ("Bob's Team", &teams[1]),
        TextLog::default(),
    )
    .unwrap();
    b.emit_opening_log();
    assert_eq!(b.log.entries[2], "|player|p1|Zoë Ünï||");
    assert_eq!(b.log.entries[3], "|player|p2|Bob's Team||");
}

#[test]
fn logical_counts_match_between_sinks() {
    let (mut quiet, mut text) = pair();
    quiet.emit_opening_log();
    text.emit_opening_log();
    let w = mon(0, 0);
    // Every operation applied to both instantiations; the logical count never depends on ENABLED.
    macro_rules! both {
        ($b:ident => $e:expr) => {{
            {
                let $b = &mut quiet;
                $e;
            }
            {
                let $b = &mut text;
                $e;
            }
        }};
    }
    both!(b => b.add(LogEntry::new("", &[], &[])));
    both!(b => b.add(LogEntry::new("t:", &[LogArg::Empty], &[])));
    both!(b => b.add(LogEntry::split("-damage", &[LogArg::Mon(w), LogArg::Health(w)], &[], SideId(0), false)));
    both!(b => b.add_move(LogEntry::new("move", &[LogArg::Mon(w), LogArg::Text("Protect")], &[])));
    both!(b => b.attr_last_move(MoveLineEdit::Still));
    both!(b => b.attr_last_move(MoveLineEdit::Miss));
    both!(b => b.hint(LogArg::Text("Fake Out only works on your first turn out."), false, None));
    both!(b => b.hint(LogArg::Text("Fake Out only works on your first turn out."), false, None));
    both!(b => b.hint(LogArg::Text("secret"), false, Some(SideId(1))));
    assert_eq!(quiet.scratch.unsent_lines, 4 + 1 + 1 + 3 + 1 + 1 + 1 + 3);
    assert_eq!(text.scratch.unsent_lines, quiet.scratch.unsent_lines);
    assert_eq!(text.scratch.unsent_lines as usize, text.log.entries.len());
}

#[test]
fn send_updates_resets_the_logical_count_for_every_sink() {
    let (mut quiet, mut text) = pair();
    quiet.emit_opening_log();
    text.emit_opening_log();
    quiet.send_updates();
    text.send_updates();
    assert_eq!(quiet.scratch.unsent_lines, 0);
    assert_eq!(text.scratch.unsent_lines, 0);
    // The text log keeps its history and the count restarts from zero.
    assert_eq!(text.log.entries.len(), 4);
    for _ in 0..1000 {
        quiet.line();
        text.line();
    }
    assert_eq!(quiet.scratch.unsent_lines, 1000);
    assert_eq!(text.scratch.unsent_lines, 1000);
    quiet.line();
    text.line();
    assert!(
        quiet.scratch.unsent_lines > 1000,
        "LINE LIMIT is `> 1000` unsent lines"
    );
    assert!(text.scratch.unsent_lines > 1000);
    quiet.send_updates();
    assert_eq!(quiet.scratch.unsent_lines, 0);
    // Draining the text log is independent of send_updates.
    let mut out = Vec::new();
    text.drain_log(&mut out);
    assert_eq!(out.len(), 1005);
    assert_eq!(text.scratch.unsent_lines, 1001);
}

#[test]
fn drain_returns_new_entries_only_and_keeps_history() {
    let mut b = standard_active();
    let mut out = Vec::new();
    b.drain_log(&mut out);
    assert!(out.is_empty());
    add(&mut b, "turn", &[LogArg::Number(1)], &[]);
    add(&mut b, "upkeep", &[], &[]);
    b.drain_log(&mut out);
    assert_eq!(out, ["|turn|1", "|upkeep"]);
    b.drain_log(&mut out);
    assert_eq!(out.len(), 2, "nothing new");
    add_split(&mut b, "-hint", &[LogArg::Text("x")], &[], 0, true);
    b.drain_log(&mut out);
    assert_eq!(&out[2..], ["|split|p1", "|-hint|x", ""]);
    assert_eq!(b.log.entries.len(), 5);
    // A fresh vector drained later still only sees new entries.
    let mut again = Vec::new();
    b.drain_log(&mut again);
    assert!(again.is_empty());
}

#[test]
fn hints_once_per_side_and_plain() {
    let mut b = standard_active();
    let text = "Illusion Level Mod is active, so this Pokémon's true level was hidden.";
    // `once` hints are remembered (data/abilities.ts:2085): printed once per battle.
    b.hint(LogArg::Text(text), true, None);
    b.hint(LogArg::Text(text), true, None);
    assert_eq!(lines(&b), [format!("|-hint|{text}")]);
    assert!(b.state.illusion_hint);
    assert_eq!(b.scratch.unsent_lines, 5, "the skipped hint does not count");
    // Plain hints print every time (Fake Out's hint appears up to 10 times in a battle).
    let fake = "Fake Out only works on your first turn out.";
    b.hint(LogArg::Text(fake), false, None);
    b.hint(LogArg::Text(fake), false, None);
    assert_eq!(b.log.entries.len(), 3);
    assert_eq!(last(&b), format!("|-hint|{fake}"));
    // A side makes it a secret-only split: the public entry is the empty string.
    b.hint(LogArg::Text("only you"), false, Some(SideId(1)));
    assert_eq!(&b.log.entries[3..], ["|split|p2", "|-hint|only you", ""]);
    assert_eq!(b.scratch.unsent_lines, 4 + 1 + 1 + 1 + 3);
    // Interpolated hint text (data/moves.ts:20930 style) through Parts.
    b.hint(
        LogArg::Parts(&[
            LogArg::Text("Special"),
            LogArg::Text("|"),
            LogArg::Text("Physical Shell Side Arm"),
        ]),
        false,
        None,
    );
    assert_eq!(last(&b), "|-hint|Special|Physical Shell Side Arm");
}

#[test]
#[should_panic(expected = "once-hint with a side")]
fn once_hint_with_a_side_is_not_modelled() {
    let mut b = standard_active();
    b.hint(LogArg::Text("x"), true, Some(SideId(0)));
}

#[test]
#[should_panic(expected = "HP-dependent argument but no split side")]
fn hp_arguments_require_a_split_entry() {
    let mut b = standard_active();
    b.add(LogEntry::new(
        "-damage",
        &[LogArg::Mon(mon(0, 0)), LogArg::Health(mon(0, 0))],
        &[],
    ));
}
